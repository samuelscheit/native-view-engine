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
    let mut border_uses_current_color = false;
    if key == "style" {
        for decl in value.split(';') {
            if let Some((raw_k, raw_v)) = decl.split_once(':') {
                let k = raw_k.trim();
                let v = raw_v.trim();
                if k.is_empty() || v.is_empty() {
                    continue;
                }
                if !apply_typed_attr_single(
                    layout,
                    style,
                    k,
                    v,
                    &mut border_uses_current_color,
                    &mut on_ext,
                ) && let Some(canonical) = canonical_raw_style_key(k)
                {
                    on_ext(ExtStyleDecl::Raw(canonical, v));
                }
            }
        }
        if border_uses_current_color {
            style.border_color = style.text_color;
        }
        return true;
    }
    let handled = apply_typed_attr_single(
        layout,
        style,
        key,
        value,
        &mut border_uses_current_color,
        &mut on_ext,
    );
    if border_uses_current_color {
        style.border_color = style.text_color;
    }
    handled
}

fn apply_typed_attr_single<F>(
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    key: &str,
    value: &str,
    border_uses_current_color: &mut bool,
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
            if let Some((width, color)) = css_shorthand::parse_border(value) {
                style.border_width = width;
                match color {
                    Some(c) => {
                        style.border_color = c;
                        *border_uses_current_color = false;
                    }
                    None => *border_uses_current_color = true,
                }
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
            if value.trim().eq_ignore_ascii_case("currentcolor") {
                *border_uses_current_color = true;
            } else if let Some(c) = parse_color(value) {
                style.border_color = c;
                *border_uses_current_color = false;
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

fn parse_rgb_channel(token: &str) -> Option<(u8, bool)> {
    let t = token.trim();
    if let Some(pct) = t.strip_suffix('%') {
        let v = parse_f32(pct)?;
        Some((((v / 100.0).clamp(0.0, 1.0) * 255.0).round() as u8, true))
    } else {
        let v = parse_f32(t)?;
        Some((v.clamp(0.0, 255.0).round() as u8, false))
    }
}

fn parse_alpha_channel(token: &str) -> Option<u8> {
    let t = token.trim();
    let alpha = match t.strip_suffix('%') {
        Some(pct) => parse_f32(pct)? / 100.0,
        None => parse_f32(t)?,
    };
    Some((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Parses strict CSS Color 4 `rgb()` / `rgba()` arguments:
/// - Legacy comma grammar: `r, g, b` or `r, g, b, a` (all RGB channels numeric or all `%`, no `/`).
/// - Modern space grammar: `r g b` or `r g b / a` (numeric or `%` channels, at most one `/`).
fn parse_rgb_args(args: &str) -> Option<ColorRgba> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains(',') {
        if trimmed.contains('/') {
            return None;
        }
        let mut iter = trimmed.split(',');
        let (r, r_pct) = parse_rgb_channel(iter.next()?)?;
        let (g, g_pct) = parse_rgb_channel(iter.next()?)?;
        let (b, b_pct) = parse_rgb_channel(iter.next()?)?;
        if r_pct != g_pct || g_pct != b_pct {
            return None;
        }
        let a = match iter.next() {
            None => 255,
            Some(raw) => parse_alpha_channel(raw)?,
        };
        if iter.next().is_some() {
            return None;
        }
        return Some(ColorRgba::rgba(r, g, b, a));
    }
    let (rgb_part, alpha_part) = match trimmed.split_once('/') {
        Some((lhs, rhs)) => {
            if rhs.contains('/') {
                return None;
            }
            (lhs, Some(rhs))
        }
        None => (trimmed, None),
    };
    let mut rgb_iter = rgb_part.split_ascii_whitespace();
    let (r, _) = parse_rgb_channel(rgb_iter.next()?)?;
    let (g, _) = parse_rgb_channel(rgb_iter.next()?)?;
    let (b, _) = parse_rgb_channel(rgb_iter.next()?)?;
    if rgb_iter.next().is_some() {
        return None;
    }
    let a = match alpha_part {
        None => 255,
        Some(raw) => {
            let mut a_iter = raw.split_ascii_whitespace();
            let tok = a_iter.next()?;
            if a_iter.next().is_some() {
                return None;
            }
            parse_alpha_channel(tok)?
        }
    };
    Some(ColorRgba::rgba(r, g, b, a))
}

#[cfg(test)]
mod inline_style_tests;
