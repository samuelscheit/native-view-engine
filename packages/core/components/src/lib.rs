//! Typed cross-platform UI primitives (`View`, `Text`, `ScrollView`, `Pressable`, `SkiaCanvas`,
//! and RN Codegen custom components) plus ergonomic style/layout builders and direct binary FFI vector
//! draw-list IR (`NVE_DRAW1`).

pub mod canvas_ir;
pub mod client_hints;
mod css_shorthand;
pub mod extended_wire;
pub mod input_hints;
pub mod interaction;
pub mod keyboard;
pub mod layout_ext;
pub mod media;
pub mod motion;
pub mod motion_clock;
#[cfg(test)]
mod motion_tests;
pub mod native_view;
pub mod paint;
pub mod reconcile;
pub mod spring;
pub mod style_patch;
pub mod svg;
mod tracker;
pub mod transform;

pub use canvas_ir::{
    CanvasOp, DrawList, DrawListBuilder, DrawListDecoder, DrawListError, DrawListHeader,
    DrawListRecorder, ImageResizeMode, DRAW_LIST_ATTR_PREFIX, MAX_DRAW_OPS, MAX_POLYLINE_POINTS,
    NVE_DRAW_HEADER_SIZE, NVE_DRAW_MAGIC, NVE_DRAW_VERSION,
};
pub use client_hints::{
    ClientHintsSnapshot, SSR_ACCEPT_CH_HEADER, SSR_CRITICAL_CH_HEADER, SSR_VARY_HEADER,
};
pub use input_hints::{AutofillHint, InputMode};
pub use interaction::{
    resolve_cursor, resolve_layout_and_style, resolve_patch, resolve_style, CursorKind,
    FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
pub use keyboard::{
    effective_bottom_inset, keyboard_insets_from_visual_viewport, KeyboardInsets,
    VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX,
};
pub use media::{
    MediaColorScheme, MediaEnv, MediaPointer, MediaQuery, ResponsiveRule, ResponsiveStylePatch,
    MEDIA_FLAG_HAS_MAX_WIDTH, MEDIA_FLAG_HAS_MIN_WIDTH, MEDIA_FLAG_HAS_REDUCED_MOTION,
    MEDIA_FLAG_REDUCED_MOTION_VAL,
};
pub use motion::{
    lerp_color_rgba, lerp_f32, lerp_style_property, lerp_style_props, resolve_transition_for,
    EasingCurve, MotionClock, MotionCompletion, MotionGroup, MotionKey, MotionTrack,
    TimedTransition, TransitionProperty, TransitionSpec,
};
pub use reconcile::{
    reconcile_three_way, ConflictPolicy, DivergenceKind, LogicalClock, ReconcileBatch,
    ReconcileDecision, StateReconciler, ThreeWayEntry,
};
pub use spring::{
    MotionPreference, SpringSpec, SpringState, DEFAULT_REST_DISPLACEMENT, DEFAULT_REST_VELOCITY,
};
pub use layout_ext::{
    compute_sticky_offset, GridItemPlacement, GridLayout, GridLine, GridSingleTrack, GridTrack,
    GridTrackPlacement, GridTrackUnit, LayoutDirection, LayoutExt, LogicalInlineEdges,
    PositionInsets, PositionMode,
};
pub use paint::{
    lerp_color_premul, normalize_stops, sample_gradient_stops, sample_normalized_stops,
    stops_are_normalized, write_css_color, write_css_gradient, BackgroundFill, GradientStop,
    LinearGradient, RadialGradient,
};
pub use style_patch::{Display, FlexWrap, Overflow, StylePatch, TextAlign};
pub use svg::{
    compute_svg_content_hash, parse_svg_path_data, write_path_data_into, FillRule, SvgPicture,
    VectorPathCmd, VectorShape, MAX_SVG_PATH_COMMANDS, MAX_SVG_PATH_INPUT_BYTES,
};
pub use transform::{
    affine_concat, affine_inverse_map_point, affine_map_point, lerp_transform_2d, Transform2D,
};

use nve_abi::num::parse_f32;
use nve_abi::{
    ColorRgba, ComponentKind, Dimension, FlexAlign, FlexDirection, FlexJustify,
    LayoutProps, StyleProps,
};

/// Resolves an HTML/RN tag name (from Dioxus `rsx!` or custom frontends) to a typed `ComponentKind`.
pub fn resolve_tag(tag: &str) -> (ComponentKind, Option<&str>) {
    match tag {
        "root" => (ComponentKind::Root, None),
        "view" | "div" | "section" | "main" | "header" | "footer" | "dialog" | "details"
        | "summary" | "form" => (ComponentKind::View, None),
        "text" | "span" | "p" | "h1" | "h2" | "h3" | "label" => (ComponentKind::Text, None),
        "scrollview" | "scroll" => (ComponentKind::ScrollView, None),
        "image" | "img" => (ComponentKind::Image, None),
        "textinput" | "text_input" | "input" | "textarea" => (ComponentKind::TextInput, None),
        "pressable" | "button" => (ComponentKind::Pressable, None),
        "skia-canvas" | "canvas" => (ComponentKind::SkiaCanvas, None),
        other => (ComponentKind::Custom, Some(other)),
    }
}

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
        _ => None,
    }
}

/// A parsed inline declaration that lives outside the binary `LayoutProps` / `StyleProps`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtStyleDecl<'a> {
    /// Typed [`LayoutExt::flex_basis`] from `flex-basis` or the basis of a `flex` shorthand
    /// (`flex: none` / `auto` / `initial` yield `Dimension::Auto`).
    FlexBasis(Dimension),
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

/// Parses inline/shorthand CSS-like or RN-like attributes into typed `LayoutProps` and
/// `StyleProps`, invoking `on_ext` for every recognized declaration outside the binary ABI
/// (typed [`ExtStyleDecl::FlexBasis`] or a raw [`ExtStyleDecl::Raw`] declaration).
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
    if key == "style" {
        for decl in value.split(';') {
            if let Some((raw_k, raw_v)) = decl.split_once(':') {
                let k = raw_k.trim();
                let v = raw_v.trim();
                if k.is_empty() || v.is_empty() {
                    continue;
                }
                if !apply_typed_attr_single(layout, style, k, v, &mut on_ext)
                    && let Some(canonical) = canonical_raw_style_key(k)
                {
                    on_ext(ExtStyleDecl::Raw(canonical, v));
                }
            }
        }
        return true;
    }
    apply_typed_attr_single(layout, style, key, value, &mut on_ext)
}

fn apply_typed_attr_single<F>(
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    key: &str,
    value: &str,
    on_ext: &mut F,
) -> bool
where
    F: FnMut(ExtStyleDecl<'_>),
{
    // CSS drops negative padding; negative margins are valid.
    if let Some(suffix) = key.strip_prefix("padding")
        && !suffix.is_empty()
    {
        return css_shorthand::apply_edge_longhand(&mut layout.padding, suffix, value, false);
    }
    if let Some(suffix) = key.strip_prefix("margin")
        && !suffix.is_empty()
    {
        return css_shorthand::apply_edge_longhand(&mut layout.margin, suffix, value, true);
    }
    match key {
        "width" => layout.width = parse_dim(value),
        "height" => layout.height = parse_dim(value),
        "min-width" | "minWidth" => layout.min_width = parse_dim(value),
        "min-height" | "minHeight" => layout.min_height = parse_dim(value),
        "max-width" | "maxWidth" => layout.max_width = parse_dim(value),
        "max-height" | "maxHeight" => layout.max_height = parse_dim(value),
        "flex-direction" | "flexDirection" => {
            layout.flex_direction = match value {
                "row" => FlexDirection::Row,
                "column-reverse" | "columnReverse" => FlexDirection::ColumnReverse,
                "row-reverse" | "rowReverse" => FlexDirection::RowReverse,
                _ => FlexDirection::Column,
            };
        }
        "justify-content" | "justifyContent" => {
            layout.justify_content = match value {
                "center" => FlexJustify::Center,
                "flex-end" | "end" => FlexJustify::FlexEnd,
                "space-between" | "spaceBetween" => FlexJustify::SpaceBetween,
                "space-around" | "spaceAround" => FlexJustify::SpaceAround,
                "space-evenly" | "spaceEvenly" => FlexJustify::SpaceEvenly,
                _ => FlexJustify::FlexStart,
            };
        }
        "align-items" | "alignItems" => {
            layout.align_items = match value {
                "flex-start" | "start" => FlexAlign::FlexStart,
                "center" => FlexAlign::Center,
                "flex-end" | "end" => FlexAlign::FlexEnd,
                "baseline" => FlexAlign::Baseline,
                _ => FlexAlign::Stretch,
            };
        }
        "flex" => {
            if let Some(flex) = css_shorthand::parse_flex(value) {
                layout.flex_grow = flex.grow;
                layout.flex_shrink = flex.shrink;
                on_ext(ExtStyleDecl::FlexBasis(flex.basis));
            }
        }
        "flex-basis" | "flexBasis" => {
            if let Some(basis) = css_shorthand::parse_flex_basis(value) {
                on_ext(ExtStyleDecl::FlexBasis(basis));
            }
        }
        "flex-grow" | "flexGrow" => {
            if let Some(v) = parse_f32(value) {
                layout.flex_grow = v;
            }
        }
        "flex-shrink" | "flexShrink" => {
            if let Some(v) = parse_f32(value) {
                layout.flex_shrink = v;
            }
        }
        "gap" => {
            if let Some(v) = parse_px(value) {
                layout.gap = v;
            }
        }
        "padding" => {
            if let Some(edges) = css_shorthand::parse_edges(value, false) {
                layout.padding = edges;
            }
        }
        "margin" => {
            if let Some(edges) = css_shorthand::parse_edges(value, true) {
                layout.margin = edges;
            }
        }
        "border" => {
            if let Some((width, color)) = css_shorthand::parse_border(value, style.text_color) {
                style.border_width = width;
                style.border_color = color;
            }
        }
        "background" | "background-color" | "backgroundColor" => {
            if let Some(c) = parse_color(value) {
                style.background = c;
            }
        }
        "color" => {
            if let Some(c) = parse_color(value) {
                style.text_color = c;
            }
        }
        "border-color" | "borderColor" => {
            if let Some(c) = parse_color(value) {
                style.border_color = c;
            }
        }
        "border-width" | "borderWidth" => {
            if let Some(v) = parse_px(value) {
                style.border_width = v;
            }
        }
        "border-radius" | "borderRadius" => {
            if let Some(v) = parse_px(value) {
                style.border_radius = v;
            }
        }
        "opacity" => {
            if let Some(v) = parse_f32(value) {
                style.opacity = v.clamp(0.0, 1.0);
            }
        }
        "font-size" | "fontSize" => {
            if let Some(v) = parse_px(value) {
                style.font_size = v;
            }
        }
        "z-index" | "zIndex" => {
            if let Ok(v) = value.parse::<i32>() {
                style.z_index = v;
            }
        }
        _ => return false,
    }
    true
}

/// Parses a CSS pixel length (`12px`, `12PX`) or a unitless RN-style number (`12`).
pub fn parse_px(raw: &str) -> Option<f32> {
    let s = raw.trim();
    let num = match s.len().checked_sub(2) {
        Some(at) if s.is_char_boundary(at) && s[at..].eq_ignore_ascii_case("px") => &s[..at],
        _ => s,
    };
    parse_f32(num)
}

pub fn parse_dim(raw: &str) -> Dimension {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("auto") {
        Dimension::Auto
    } else if let Some(pct) = s.strip_suffix('%') {
        parse_f32(pct.trim())
            .map(Dimension::Percent)
            .unwrap_or(Dimension::Auto)
    } else {
        parse_px(s).map(Dimension::Points).unwrap_or(Dimension::Auto)
    }
}

pub fn parse_color(raw: &str) -> Option<ColorRgba> {
    let s = raw.trim();
    if let Some(hex) = s.strip_prefix('#') {
        match hex.len() {
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
        }
    } else if let Some(args) = strip_ascii_prefix(s, "rgba(").or_else(|| strip_ascii_prefix(s, "rgb(")) {
        parse_rgb_args(args.strip_suffix(')')?)
    } else {
        match s.to_ascii_lowercase().as_str() {
            "white" => Some(ColorRgba::WHITE),
            "black" => Some(ColorRgba::BLACK),
            "transparent" => Some(ColorRgba::TRANSPARENT),
            "red" => Some(ColorRgba::rgb(239, 68, 68)),
            "green" => Some(ColorRgba::rgb(34, 197, 94)),
            "blue" => Some(ColorRgba::rgb(59, 130, 246)),
            _ => None,
        }
    }
}

fn strip_ascii_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// `r, g, b[, a]` or `r g b[ / a]` (`rgb()` and `rgba()` are aliases): channels are numbers
/// clamped to `0..=255`, alpha a number or percentage clamped to `0..=1`.
fn parse_rgb_args(args: &str) -> Option<ColorRgba> {
    let mut parts = args
        .split(|c: char| c == ',' || c == '/' || c.is_ascii_whitespace())
        .filter(|t| !t.is_empty());
    let mut rgb = [0u8; 3];
    for channel in &mut rgb {
        *channel = parse_f32(parts.next()?)?.clamp(0.0, 255.0).round() as u8;
    }
    let alpha = match parts.next() {
        None => 1.0,
        Some(a) => match a.strip_suffix('%') {
            Some(pct) => parse_f32(pct)? / 100.0,
            None => parse_f32(a)?,
        },
    };
    if parts.next().is_some() {
        return None;
    }
    let [r, g, b] = rgb;
    Some(ColorRgba::rgba(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::EdgeInsets;

    /// Collects every [`ExtStyleDecl`] emitted while applying `key: value`.
    fn ext_decls(
        l: &mut LayoutProps,
        s: &mut StyleProps,
        key: &str,
        value: &str,
    ) -> Vec<(Option<Dimension>, String)> {
        let mut out = Vec::new();
        assert!(apply_typed_attr_with_ext(l, s, key, value, |d| out.push(match d {
            ExtStyleDecl::FlexBasis(b) => (Some(b), String::new()),
            ExtStyleDecl::Raw(k, v) => (None, format!("{k}:{v}")),
        })));
        out
    }

    #[test]
    fn inline_style_shorthands_fill_typed_props_and_typed_flex_basis() {
        let mut l = LayoutProps::default();
        let mut s = StyleProps::default();
        let decls = ext_decls(
            &mut l,
            &mut s,
            "style",
            "padding: 14px 16px; margin: 1px 2px 3px; margin-left: -9px; paddingTop: 4PX; border: 1px solid currentColor; color: rgb(30, 41, 59); flex: 1 1 280px;",
        );
        let edges = |top, right, bottom, left| EdgeInsets { top, right, bottom, left };
        let slate = ColorRgba::rgb(0x1e, 0x29, 0x3b);
        assert_eq!(l.padding, edges(4.0, 16.0, 14.0, 16.0));
        assert_eq!(l.margin, edges(1.0, 2.0, 3.0, -9.0), "negative margins are valid");
        // `currentColor` resolves against the color in effect when `border` is parsed.
        assert_eq!((s.border_width, s.border_color), (1.0, ColorRgba::BLACK));
        assert_eq!(s.text_color, slate);
        assert_eq!((l.flex_grow, l.flex_shrink), (1.0, 1.0));
        assert_eq!(decls, vec![(Some(Dimension::Points(280.0)), String::new())]);

        // `flex: <grow>` implies basis 0%; `flex: none` resets factors and basis; unitless zero.
        let decls = ext_decls(&mut l, &mut s, "flex", "2");
        assert_eq!((l.flex_grow, l.flex_shrink), (2.0, 1.0));
        assert_eq!(decls, vec![(Some(Dimension::Percent(0.0)), String::new())]);
        let decls = ext_decls(&mut l, &mut s, "style", "flex: none; flexBasis: 12px; flex-basis: 0");
        assert_eq!((l.flex_grow, l.flex_shrink), (0.0, 0.0));
        let bases: Vec<_> = decls.into_iter().map(|(b, _)| b.unwrap()).collect();
        assert_eq!(
            bases,
            [Dimension::Auto, Dimension::Points(12.0), Dimension::Points(0.0)]
        );
        let decls = ext_decls(&mut l, &mut s, "style", "flex: 0 0 0");
        assert_eq!((l.flex_grow, l.flex_shrink), (0.0, 0.0));
        assert_eq!(decls, vec![(Some(Dimension::Points(0.0)), String::new())]);
        // Unsupported bases (`content`, unitless non-zero, negative) emit nothing.
        assert!(ext_decls(&mut l, &mut s, "style", "flex-basis: content; flex-basis: 12; flex-basis: -1px").is_empty());

        // Invalid shorthands leave every target untouched (all-or-nothing).
        let before = (l, s);
        assert!(ext_decls(&mut l, &mut s, "style", "padding: 1px 2px 3px 4px 5px; padding: -1px 2px; paddingTop: -4px; margin: 0 auto; border: 1px wavy red; flex: 1 280px 1; flex: 1 2 3;").is_empty());
        assert_eq!((l, s), before);
    }

    #[test]
    fn parses_inline_style_and_colors() {
        let mut l = LayoutProps::default();
        let mut s = StyleProps::default();
        assert!(apply_typed_attr(
            &mut l,
            &mut s,
            "style",
            "width: 100%; height: 64px; flex-direction: row; gap: 12; padding: 16px; background-color: #0f172a; color: #ffffff; border-radius: 8px;"
        ));
        assert_eq!(l.width, Dimension::Percent(100.0));
        assert_eq!(l.height, Dimension::Points(64.0));
        assert_eq!(l.flex_direction, FlexDirection::Row);
        assert_eq!(l.gap, 12.0);
        assert_eq!(l.padding, EdgeInsets::all(16.0));
        assert_eq!(s.background, ColorRgba::rgb(15, 23, 42));
        assert_eq!(s.text_color, ColorRgba::WHITE);
        assert_eq!(s.border_radius, 8.0);
    }

    #[test]
    fn numeric_style_values_use_the_fixed_point_parser() {
        assert_eq!(parse_px("12px"), Some(12.0));
        assert_eq!(parse_px("12PX"), Some(12.0));
        assert_eq!(parse_px(" 12Px "), Some(12.0));
        assert_eq!(parse_px(" 12.5 "), Some(12.5));
        assert_eq!(parse_px("-0px").map(f32::to_bits), Some((-0.0f32).to_bits()));
        assert_eq!(parse_px("1e2px"), Some(100.0));
        assert_eq!(parse_px(".5"), Some(0.5));
        for bad in ["", "px", "12 px", "12pt", "inf", "NaN", "1e39", "0x10", "1,5px", "--1"] {
            assert_eq!(parse_px(bad), None, "{bad:?}");
        }
        assert_eq!(parse_dim("33.5%"), Dimension::Percent(33.5));
        assert_eq!(parse_dim(" 50 % "), Dimension::Percent(50.0));
        assert_eq!(parse_dim("AUTO"), Dimension::Auto);
        assert_eq!(parse_dim("abc%"), Dimension::Auto);
        assert_eq!(parse_dim("7px"), Dimension::Points(7.0));

        let mut l = LayoutProps::default();
        let mut s = StyleProps::default();
        assert!(apply_typed_attr(&mut l, &mut s, "flex-grow", "2.5"));
        assert!(apply_typed_attr(&mut l, &mut s, "flexShrink", "0"));
        assert!(apply_typed_attr(&mut l, &mut s, "opacity", "1.5"));
        assert_eq!((l.flex_grow, l.flex_shrink, s.opacity), (2.5, 0.0, 1.0));
        // Malformed numbers leave the previous value untouched.
        assert!(apply_typed_attr(&mut l, &mut s, "flex-grow", "fast"));
        assert!(apply_typed_attr(&mut l, &mut s, "opacity", "nan"));
        assert_eq!((l.flex_grow, s.opacity), (2.5, 1.0));
    }

    #[test]
    fn forwards_non_abi_inline_style_declarations_as_raw_props() {
        let mut l = LayoutProps::default();
        let mut s = StyleProps::default();
        let raw = |decls: Vec<(Option<Dimension>, String)>| -> Vec<String> {
            decls.into_iter().map(|(_, kv)| kv).collect()
        };
        let forwarded = raw(ext_decls(
            &mut l,
            &mut s,
            "style",
            "font-size: 18px; font-weight: 700; text-align: center; box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35); overflow: hidden; unknown-prop: 42;",
        ));
        assert_eq!(s.font_size, 18.0);
        assert_eq!(
            forwarded,
            [
                "font-weight:700",
                "text-align:center",
                "box-shadow:0 4px 12px rgba(0, 0, 0, 0.35)",
                "overflow:hidden",
            ]
        );

        // CamelCase variants inside inline style normalize to canonical kebab-case keys.
        let camel_forwarded = raw(ext_decls(
            &mut l,
            &mut s,
            "style",
            "fontWeight: semibold; textAlign: right; boxShadow: 2px 3px 6px #00000080;",
        ));
        assert_eq!(
            camel_forwarded,
            [
                "font-weight:semibold",
                "text-align:right",
                "box-shadow:2px 3px 6px #00000080",
            ]
        );
    }

    #[test]
    fn rgb_functions_parse_legacy_and_modern_syntax() {
        assert_eq!(parse_color("rgb(30, 41, 59)"), Some(ColorRgba::rgb(30, 41, 59)));
        assert_eq!(parse_color("RGBA(0,0,0,0.5)"), Some(ColorRgba::rgba(0, 0, 0, 128)));
        assert_eq!(parse_color("rgb(300 -5 10.4 / 25%)"), Some(ColorRgba::rgba(255, 0, 10, 64)));
        assert_eq!(parse_color("rgba(1, 2, 3, 2)"), Some(ColorRgba::rgb(1, 2, 3)), "alpha clamps");
        for bad in ["rgb(1, 2)", "rgb(1, 2, 3", "rgb(1, 2, 3, 4, 5)", "rgb(a, b, c)", "rgb()"] {
            assert_eq!(parse_color(bad), None, "{bad:?}");
        }
    }
}
