//! Typed cross-platform UI primitives (`View`, `Text`, `ScrollView`, `Pressable`, `SkiaCanvas`,
//! and RN Codegen custom components) plus ergonomic style/layout builders.

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
        "textinput" | "input" | "textarea" => (ComponentKind::TextInput, None),
        "pressable" | "button" => (ComponentKind::Pressable, None),
        "skia-canvas" | "canvas" => (ComponentKind::SkiaCanvas, None),
        other => (ComponentKind::Custom, Some(other)),
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
            if let Ok(v) = value.parse::<f32>() {
                layout.flex_grow = v;
            }
        }
        "flex-shrink" | "flexShrink" => {
            if let Ok(v) = value.parse::<f32>() {
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
            if let Ok(v) = value.parse::<f32>() {
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
        "style" => {
            for decl in value.split(';') {
                if let Some((k, v)) = decl.split_once(':') {
                    apply_typed_attr(layout, style, k.trim(), v.trim());
                }
            }
        }
        _ => return false,
    }
    true
}

pub fn parse_px(raw: &str) -> Option<f32> {
    let s = raw.trim().strip_suffix("px").unwrap_or(raw.trim());
    s.parse::<f32>().ok()
}

pub fn parse_dim(raw: &str) -> Dimension {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("auto") {
        Dimension::Auto
    } else if let Some(pct) = s.strip_suffix('%') {
        pct.trim()
            .parse::<f32>()
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
}
