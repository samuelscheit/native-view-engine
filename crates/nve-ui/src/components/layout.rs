//! Layout primitives: [`Stack`], [`Row`], [`Col`], [`Separator`], and [`Card`].

use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;

use crate::focus::Orientation;
use crate::style_buf::{StyleBuf, SurfaceStyle};
use crate::theme::use_theme;

/// Visual elevation and border style for [`Card`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardVariant {
    #[default]
    Elevated,
    Outlined,
    Subtle,
}

/// Flex stack container supporting horizontal or vertical layout with theme-aware spacing.
#[component]
pub fn Stack(
    #[props(default)] direction: Orientation,
    #[props(default)] gap: Option<f32>,
    #[props(default)] padding: Option<f32>,
    #[props(default)] align: Option<String>,
    #[props(default)] justify: Option<String>,
    #[props(default)] style: Option<String>,
    #[props(default)] test_id: Option<String>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut sb = StyleBuf::new();
    sb.flex_direction(match direction {
        Orientation::Horizontal => "row",
        Orientation::Vertical => "column",
    })
    .gap(gap.unwrap_or(theme.spacing.sm));

    if let Some(p) = padding {
        sb.padding(p);
    }
    if let Some(ref a) = align {
        sb.align_items(a);
    }
    if let Some(ref j) = justify {
        sb.justify_content(j);
    }
    if let Some(ref extra) = style {
        sb.raw(extra);
    }

    let style_str = sb.finish();
    rsx! {
        view {
            test_id: test_id,
            style: "{style_str}",
            {children}
        }
    }
}

/// Horizontal flex container (`flex-direction: row`, centered cross-axis by default).
#[component]
pub fn Row(
    #[props(default)] gap: Option<f32>,
    #[props(default)] padding: Option<f32>,
    #[props(default)] align: Option<String>,
    #[props(default)] justify: Option<String>,
    #[props(default)] style: Option<String>,
    #[props(default)] test_id: Option<String>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut sb = StyleBuf::new();
    sb.flex_direction("row")
        .align_items(align.as_deref().unwrap_or("center"))
        .gap(gap.unwrap_or(theme.spacing.sm));

    if let Some(p) = padding {
        sb.padding(p);
    }
    if let Some(ref j) = justify {
        sb.justify_content(j);
    }
    if let Some(ref extra) = style {
        sb.raw(extra);
    }

    let style_str = sb.finish();
    rsx! {
        view {
            test_id: test_id,
            style: "{style_str}",
            {children}
        }
    }
}

/// Vertical flex container (`flex-direction: column`).
#[component]
pub fn Col(
    #[props(default)] gap: Option<f32>,
    #[props(default)] padding: Option<f32>,
    #[props(default)] align: Option<String>,
    #[props(default)] justify: Option<String>,
    #[props(default)] style: Option<String>,
    #[props(default)] test_id: Option<String>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut sb = StyleBuf::new();
    sb.flex_direction("column")
        .gap(gap.unwrap_or(theme.spacing.sm));

    if let Some(p) = padding {
        sb.padding(p);
    }
    if let Some(ref a) = align {
        sb.align_items(a);
    }
    if let Some(ref j) = justify {
        sb.justify_content(j);
    }
    if let Some(ref extra) = style {
        sb.raw(extra);
    }

    let style_str = sb.finish();
    rsx! {
        view {
            test_id: test_id,
            style: "{style_str}",
            {children}
        }
    }
}

/// Accessible divider rule (`accessibilityRole="separator"`).
#[component]
pub fn Separator(
    #[props(default)] orientation: Orientation,
    #[props(default)] margin: Option<f32>,
    #[props(default)] style: Option<String>,
    #[props(default)] test_id: Option<String>,
) -> Element {
    let theme = use_theme();
    let mut sb = StyleBuf::new();
    match orientation {
        Orientation::Horizontal => {
            sb.width_pct(100.0).height_px(1.0);
        }
        Orientation::Vertical => {
            sb.width_px(1.0).height_pct(100.0);
        }
    }
    if let Some(m) = margin {
        sb.margin(m);
    }
    sb.surface(SurfaceStyle {
        background: theme.palette.border.as_ref(),
        border_width: 0.0,
        border_color: "transparent",
        border_radius: 0.0,
        opacity: 1.0,
        box_shadow: "none",
    });
    if let Some(ref extra) = style {
        sb.raw(extra);
    }

    let style_str = sb.finish();
    rsx! {
        view {
            test_id: test_id,
            accessibility_role: "separator",
            style: "{style_str}",
        }
    }
}

/// Elevated, outlined, or subtle surface card container with explicit full-property styling.
#[component]
pub fn Card(
    #[props(default)] variant: CardVariant,
    #[props(default)] padding: Option<f32>,
    #[props(default)] gap: Option<f32>,
    #[props(default)] style: Option<String>,
    #[props(default)] test_id: Option<String>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let (bg, border_width, border_color, shadow) = match variant {
        CardVariant::Elevated => (
            theme.palette.surface_elevated.as_ref(),
            if theme.is_high_contrast() { 2.0 } else { 1.0 },
            theme.palette.border.as_ref(),
            theme.elevation_shadow(),
        ),
        CardVariant::Outlined => (
            theme.palette.surface.as_ref(),
            if theme.is_high_contrast() { 2.0 } else { 1.0 },
            theme.palette.border.as_ref(),
            "none",
        ),
        CardVariant::Subtle => (
            theme.palette.surface_subtle.as_ref(),
            if theme.is_high_contrast() { 1.0 } else { 0.0 },
            if theme.is_high_contrast() {
                theme.palette.border.as_ref()
            } else {
                "transparent"
            },
            "none",
        ),
    };

    let mut sb = StyleBuf::new();
    sb.flex_direction("column")
        .padding(padding.unwrap_or(theme.spacing.lg))
        .gap(gap.unwrap_or(theme.spacing.sm))
        .surface(SurfaceStyle {
            background: bg,
            border_width,
            border_color,
            border_radius: theme.radii.lg,
            opacity: 1.0,
            box_shadow: shadow,
        });
    if let Some(ref extra) = style {
        sb.raw(extra);
    }

    let style_str = sb.finish();
    rsx! {
        view {
            test_id: test_id,
            style: "{style_str}",
            {children}
        }
    }
}
