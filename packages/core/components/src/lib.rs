//! Typed cross-platform UI primitives (`View`, `Text`, `ScrollView`, `Pressable`, `SkiaCanvas`,
//! and RN Codegen custom components) plus ergonomic style/layout builders and zero-JSON vector
//! draw-list IR (`NVE_DRAW1`).

pub mod canvas_ir;
pub mod client_hints;
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
pub mod reconcile;
pub mod spring;
pub mod style_patch;
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
pub use style_patch::{Display, FlexWrap, Overflow, StylePatch, TextAlign};
pub use transform::{
    affine_concat, affine_inverse_map_point, affine_map_point, lerp_transform_2d, Transform2D,
};

use nve_abi::num::parse_f32;
use nve_abi::{
    ColorRgba, ComponentKind, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify,
    LayoutProps, StyleProps,
};

/// Resolves an HTML/RN tag name (from Dioxus `rsx!` or custom frontends) to a typed `ComponentKind`.
pub fn resolve_tag(tag: &str) -> (ComponentKind, Option<&str>) {
    match tag {
        "root" => (ComponentKind::Root, None),
        "view" | "div" | "section" | "main" | "header" | "footer" => (ComponentKind::View, None),
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
        "font-weight" | "fontWeight" => Some("font-weight"),
        "text-align" | "textAlign" => Some("text-align"),
        "box-shadow" | "boxShadow" => Some("box-shadow"),
        "overflow" => Some("overflow"),
        "flex-wrap" | "flexWrap" => Some("flex-wrap"),
        _ => None,
    }
}

/// Parses inline/shorthand CSS-like or RN-like attribute key/value pairs directly into typed
/// `LayoutProps` and `StyleProps` — avoiding `folly::dynamic` string parsing in C++.
pub fn apply_typed_attr(
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    key: &str,
    value: &str,
) -> bool {
    apply_typed_attr_with_raw(layout, style, key, value, |_, _| {})
}

/// Parses inline/shorthand CSS-like or RN-like attributes into typed `LayoutProps` and
/// `StyleProps`, invoking `on_raw(canonical_key, value)` for recognized non-ABI inline `style`
/// declarations (`font-weight`, `text-align`, `box-shadow`, `overflow`) so callers can forward
/// them via `raw_props` with zero ABI breakage.
pub fn apply_typed_attr_with_raw<F>(
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    key: &str,
    value: &str,
    mut on_raw: F,
) -> bool
where
    F: FnMut(&str, &str),
{
    if key == "style" {
        for decl in value.split(';') {
            if let Some((raw_k, raw_v)) = decl.split_once(':') {
                let k = raw_k.trim();
                let v = raw_v.trim();
                if k.is_empty() || v.is_empty() {
                    continue;
                }
                if !apply_typed_attr_single(layout, style, k, v)
                    && let Some(canonical) = canonical_raw_style_key(k)
                {
                    on_raw(canonical, v);
                }
            }
        }
        return true;
    }
    apply_typed_attr_single(layout, style, key, value)
}

fn apply_typed_attr_single(
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    key: &str,
    value: &str,
) -> bool {
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
        "flex" | "flex-grow" | "flexGrow" => {
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
            if let Some(v) = parse_px(value) {
                layout.padding = EdgeInsets::all(v);
            }
        }
        "padding-horizontal" | "paddingHorizontal" => {
            if let Some(v) = parse_px(value) {
                layout.padding.left = v;
                layout.padding.right = v;
            }
        }
        "padding-vertical" | "paddingVertical" => {
            if let Some(v) = parse_px(value) {
                layout.padding.top = v;
                layout.padding.bottom = v;
            }
        }
        "margin" => {
            if let Some(v) = parse_px(value) {
                layout.margin = EdgeInsets::all(v);
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

pub fn parse_px(raw: &str) -> Option<f32> {
    let s = raw.trim().strip_suffix("px").unwrap_or(raw.trim());
    parse_f32(s)
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut forwarded: Vec<(String, String)> = Vec::new();
        assert!(apply_typed_attr_with_raw(
            &mut l,
            &mut s,
            "style",
            "font-size: 18px; font-weight: 700; text-align: center; box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35); overflow: hidden; unknown-prop: 42;",
            |k, v| forwarded.push((k.to_owned(), v.to_owned())),
        ));
        assert_eq!(s.font_size, 18.0);
        assert_eq!(
            forwarded,
            vec![
                ("font-weight".to_string(), "700".to_string()),
                ("text-align".to_string(), "center".to_string()),
                (
                    "box-shadow".to_string(),
                    "0 4px 12px rgba(0, 0, 0, 0.35)".to_string()
                ),
                ("overflow".to_string(), "hidden".to_string()),
            ]
        );

        // CamelCase variants inside inline style normalize to canonical kebab-case keys.
        let mut camel_forwarded: Vec<(String, String)> = Vec::new();
        assert!(apply_typed_attr_with_raw(
            &mut l,
            &mut s,
            "style",
            "fontWeight: semibold; textAlign: right; boxShadow: 2px 3px 6px #00000080;",
            |k, v| camel_forwarded.push((k.to_owned(), v.to_owned())),
        ));
        assert_eq!(
            camel_forwarded,
            vec![
                ("font-weight".to_string(), "semibold".to_string()),
                ("text-align".to_string(), "right".to_string()),
                ("box-shadow".to_string(), "2px 3px 6px #00000080".to_string()),
            ]
        );
    }
}
