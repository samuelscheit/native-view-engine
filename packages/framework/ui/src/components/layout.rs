//! Layout primitives: [`Stack`], [`Row`], [`Col`], [`Separator`], and [`Card`].

use dioxus_core_macro::Props;
use nve_abi::{ColorRgba, FlexAlign, FlexDirection};
use nve_dioxus::prelude::*;

use crate::focus::Orientation;
use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle};
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
    let mut sb = TypedStyleBuilder::new()
        .flex_direction(match direction {
            Orientation::Horizontal => FlexDirection::Row,
            Orientation::Vertical => FlexDirection::Column,
        })
        .gap(gap.unwrap_or(theme.spacing.sm));

    if let Some(p) = padding {
        sb = sb.padding(p);
    }
    if let Some(ref a) = align {
        sb = sb.align_items_str(a);
    }
    if let Some(ref j) = justify {
        sb = sb.justify_content_str(j);
    }

    rsx! {
        view {
            test_id: test_id,
            nve_style: sb.finish(),
            style: style,
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
    let mut sb = TypedStyleBuilder::new()
        .flex_row()
        .gap(gap.unwrap_or(theme.spacing.sm));
    sb = match align.as_deref() {
        Some(a) => sb.align_items_str(a),
        None => sb.align_items(FlexAlign::Center),
    };

    if let Some(p) = padding {
        sb = sb.padding(p);
    }
    if let Some(ref j) = justify {
        sb = sb.justify_content_str(j);
    }

    rsx! {
        view {
            test_id: test_id,
            nve_style: sb.finish(),
            style: style,
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
    let mut sb = TypedStyleBuilder::new()
        .flex_col()
        .gap(gap.unwrap_or(theme.spacing.sm));

    if let Some(p) = padding {
        sb = sb.padding(p);
    }
    if let Some(ref a) = align {
        sb = sb.align_items_str(a);
    }
    if let Some(ref j) = justify {
        sb = sb.justify_content_str(j);
    }

    rsx! {
        view {
            test_id: test_id,
            nve_style: sb.finish(),
            style: style,
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
    let mut sb = match orientation {
        Orientation::Horizontal => TypedStyleBuilder::new().width_pct(100.0).height_px(1.0),
        Orientation::Vertical => TypedStyleBuilder::new().width_px(1.0).height_pct(100.0),
    };
    if let Some(m) = margin {
        sb = sb.margin(m);
    }
    let sep_style = sb
        .surface(TypedSurfaceStyle {
            background: theme.border.default,
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: 0.0,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    rsx! {
        view {
            test_id: test_id,
            accessibility_role: "separator",
            nve_style: sep_style,
            style: style,
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
            theme.surface.raised,
            if theme.is_high_contrast() { 2.0 } else { 1.0 },
            theme.border.default,
            theme.elevation.raised,
        ),
        CardVariant::Outlined => (
            theme.surface.base,
            if theme.is_high_contrast() { 2.0 } else { 1.0 },
            theme.border.default,
            "none",
        ),
        CardVariant::Subtle => (
            theme.surface.subtle,
            if theme.is_high_contrast() { 1.0 } else { 0.0 },
            if theme.is_high_contrast() {
                theme.border.default
            } else {
                ColorRgba::TRANSPARENT
            },
            "none",
        ),
    };

    let card_style = TypedStyleBuilder::new()
        .flex_col()
        .padding(padding.unwrap_or(theme.spacing.lg))
        .gap(gap.unwrap_or(theme.spacing.sm))
        .surface(TypedSurfaceStyle {
            background: bg,
            border_width,
            border_color,
            border_radius: theme.radius.lg,
            opacity: 1.0,
            box_shadow: shadow,
        })
        .finish();

    rsx! {
        view {
            test_id: test_id,
            nve_style: card_style,
            style: style,
            {children}
        }
    }
}
