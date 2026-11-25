//! CSS Flexbox/style translation, HTML tag resolution, image `object-fit` mapping,
//! and the cross-target [`MirrorElement`] DOM node model.

use core::fmt::Write as _;
use std::collections::HashMap;

use nve_abi::{
    ColorRgba, ComponentKind, Dimension, EventMask, FlexAlign, FlexDirection, FlexJustify,
    LayoutProps, NodeId, RawPropValue, StyleProps,
};
use nve_components::canvas_ir::{CanvasOp, ImageResizeMode};

/// Maps `resizeMode` / `resize_mode` / `objectFit` raw prop values to CSS `object-fit` keywords.
pub fn raw_prop_to_object_fit(value: &RawPropValue) -> Option<&'static str> {
    match value {
        RawPropValue::String(raw) => {
            let trimmed = raw.trim();
            if let Some(mode) = ImageResizeMode::parse(trimmed) {
                return Some(match mode {
                    ImageResizeMode::Cover => "cover",
                    ImageResizeMode::Contain => "contain",
                    ImageResizeMode::Stretch => "fill",
                    ImageResizeMode::Center => "none",
                });
            }
            match trimmed.to_ascii_lowercase().as_str() {
                "fill" => Some("fill"),
                "none" => Some("none"),
                "scale-down" | "scaledown" => Some("scale-down"),
                _ => None,
            }
        }
        RawPropValue::I64(n) => {
            let mode = u8::try_from(*n).ok().and_then(ImageResizeMode::from_u8)?;
            Some(match mode {
                ImageResizeMode::Cover => "cover",
                ImageResizeMode::Contain => "contain",
                ImageResizeMode::Stretch => "fill",
                ImageResizeMode::Center => "none",
            })
        }
        _ => None,
    }
}

/// Returns `true` if `key` names an image fit prop (`resizeMode`, `resize_mode`, or `objectFit`).
#[inline]
pub fn is_image_fit_prop(key: &str) -> bool {
    matches!(key, "resizeMode" | "resize_mode" | "objectFit")
}

pub fn component_to_html_tag(kind: ComponentKind) -> &'static str {
    match kind {
        ComponentKind::Root | ComponentKind::View | ComponentKind::ScrollView => "div",
        ComponentKind::Text => "span",
        ComponentKind::Image => "img",
        ComponentKind::TextInput => "input",
        ComponentKind::Pressable => "button",
        ComponentKind::SkiaCanvas => "canvas",
        ComponentKind::Custom => "div",
    }
}

#[inline]
fn write_dim(d: Dimension, out: &mut String) {
    match d {
        Dimension::Auto => out.push_str("auto"),
        Dimension::Points(v) => {
            let _ = write!(out, "{v}px");
        }
        Dimension::Percent(v) => {
            let _ = write!(out, "{v}%");
        }
    }
}

#[inline]
fn write_rgba(c: ColorRgba, out: &mut String) {
    let _ = write!(
        out,
        "rgba({},{},{},{:.3})",
        c.r,
        c.g,
        c.b,
        c.a as f32 / 255.0
    );
}

/// Appends Flexbox CSS declarations for `l` directly into `out` with zero intermediate allocations.
pub fn layout_to_css_into(l: &LayoutProps, out: &mut String) {
    let dir = match l.flex_direction {
        FlexDirection::Column => "column",
        FlexDirection::Row => "row",
        FlexDirection::ColumnReverse => "column-reverse",
        FlexDirection::RowReverse => "row-reverse",
    };
    let justify = match l.justify_content {
        FlexJustify::FlexStart => "flex-start",
        FlexJustify::Center => "center",
        FlexJustify::FlexEnd => "flex-end",
        FlexJustify::SpaceBetween => "space-between",
        FlexJustify::SpaceAround => "space-around",
        FlexJustify::SpaceEvenly => "space-evenly",
    };
    let align = match l.align_items {
        FlexAlign::Stretch => "stretch",
        FlexAlign::FlexStart => "flex-start",
        FlexAlign::Center => "center",
        FlexAlign::FlexEnd => "flex-end",
        FlexAlign::Baseline => "baseline",
    };

    out.push_str("display:flex;box-sizing:border-box;flex-direction:");
    out.push_str(dir);
    out.push_str(";justify-content:");
    out.push_str(justify);
    out.push_str(";align-items:");
    out.push_str(align);
    let _ = write!(
        out,
        ";flex-grow:{};flex-shrink:{};gap:{}px;width:",
        l.flex_grow, l.flex_shrink, l.gap
    );
    write_dim(l.width, out);
    out.push_str(";height:");
    write_dim(l.height, out);
    let _ = write!(
        out,
        ";padding:{}px {}px {}px {}px;margin:{}px {}px {}px {}px;",
        l.padding.top,
        l.padding.right,
        l.padding.bottom,
        l.padding.left,
        l.margin.top,
        l.margin.right,
        l.margin.bottom,
        l.margin.left,
    );

    if l.min_width != Dimension::Auto {
        out.push_str("min-width:");
        write_dim(l.min_width, out);
        out.push(';');
    }
    if l.min_height != Dimension::Auto {
        out.push_str("min-height:");
        write_dim(l.min_height, out);
        out.push(';');
    }
    if l.max_width != Dimension::Auto {
        out.push_str("max-width:");
        write_dim(l.max_width, out);
        out.push(';');
    }
    if l.max_height != Dimension::Auto {
        out.push_str("max-height:");
        write_dim(l.max_height, out);
        out.push(';');
    }
}

pub fn layout_to_css(l: &LayoutProps) -> String {
    let mut out = String::with_capacity(256);
    layout_to_css_into(l, &mut out);
    out
}

/// Appends visual style CSS declarations for `s` directly into `out` with zero intermediate allocations.
pub fn style_to_css_into(s: &StyleProps, out: &mut String) {
    out.push_str("background-color:");
    write_rgba(s.background, out);
    out.push_str(";color:");
    write_rgba(s.text_color, out);
    let _ = write!(out, ";border:{}px solid ", s.border_width);
    write_rgba(s.border_color, out);
    let _ = write!(
        out,
        ";border-radius:{}px;opacity:{};font-size:{}px;z-index:{};",
        s.border_radius, s.opacity, s.font_size, s.z_index,
    );
}

pub fn style_to_css(s: &StyleProps) -> String {
    let mut out = String::with_capacity(192);
    style_to_css_into(s, &mut out);
    out
}

fn kind_default_css(kind: ComponentKind, event_mask: u64) -> &'static str {
    match kind {
        ComponentKind::ScrollView => "overflow:auto;",
        ComponentKind::Pressable => "cursor:pointer;appearance:none;font:inherit;",
        ComponentKind::TextInput => "font:inherit;outline:none;",
        _ if (event_mask & EventMask::PRESS) != 0 => "cursor:pointer;",
        _ => "",
    }
}

#[derive(Debug, Clone)]
pub struct MirrorElement {
    pub kind: ComponentKind,
    pub tag: &'static str,
    pub layout: LayoutProps,
    pub style: StyleProps,
    pub layout_css: String,
    pub style_css: String,
    pub patch_css: String,
    pub object_fit: Option<&'static str>,
    pub text: Option<String>,
    pub children: Vec<NodeId>,
    pub event_mask: u64,
    pub raw_props: HashMap<String, RawPropValue>,
    pub draw_ops: Vec<CanvasOp>,
    pub draw_op_count: usize,
}

impl MirrorElement {
    pub(crate) fn new(kind: ComponentKind) -> Self {
        Self {
            kind,
            tag: component_to_html_tag(kind),
            layout: LayoutProps::default(),
            style: StyleProps::default(),
            layout_css: String::new(),
            style_css: String::new(),
            patch_css: String::new(),
            object_fit: None,
            text: None,
            children: Vec::new(),
            event_mask: EventMask::NONE,
            raw_props: HashMap::new(),
            draw_ops: Vec::new(),
            draw_op_count: 0,
        }
    }

    /// Appends full inline `style` text into `out` without allocating an intermediate `String`.
    #[inline]
    pub fn combined_css_into(&self, out: &mut String) {
        out.push_str(kind_default_css(self.kind, self.event_mask));
        out.push_str(&self.layout_css);
        out.push_str(&self.style_css);
        if let Some(fit) = self.object_fit {
            out.push_str("object-fit:");
            out.push_str(fit);
            out.push(';');
        }
        out.push_str(&self.patch_css);
    }

    /// Full inline `style` text combining kind defaults, typed layout, typed style, `object-fit`,
    /// and Tier-1 patches.
    pub fn combined_css(&self) -> String {
        let base = kind_default_css(self.kind, self.event_mask);
        let fit_len = self.object_fit.map_or(0, |f| f.len() + 12);
        let mut out = String::with_capacity(
            base.len()
                + self.layout_css.len()
                + self.style_css.len()
                + fit_len
                + self.patch_css.len(),
        );
        self.combined_css_into(&mut out);
        out
    }
}
