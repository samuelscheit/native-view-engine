//! Action and display components: [`Button`], [`IconButton`], and [`Badge`].

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::{FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;

use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};
use crate::components::roles::{badge_colors, button_colors, ControlColors};
use crate::theme::{use_theme, Theme};

/// Visual hierarchy variants for [`Button`] and [`IconButton`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Ghost,
    Destructive,
    Outline,
}

/// Control sizing tokens for [`Button`] and [`IconButton`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
    Lg,
}

/// Semantic status tone for [`Badge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeVariant {
    #[default]
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

fn button_metrics(theme: &Theme, size: ButtonSize) -> (f32, f32, f32, f32) {
    let pad_scale = match theme.density {
        crate::UiDensity::Comfortable => 1.0,
        crate::UiDensity::Compact => 0.8,
        crate::UiDensity::Spacious => 1.15,
    };
    match size {
        ButtonSize::Sm => (
            30.0 * pad_scale,
            theme.spacing.sm,
            theme.spacing.xs,
            theme.typography.ui.size,
        ),
        ButtonSize::Md => (
            38.0 * pad_scale,
            theme.spacing.lg,
            theme.spacing.sm,
            theme.typography.body.size,
        ),
        ButtonSize::Lg => (
            46.0 * pad_scale,
            theme.spacing.xl,
            theme.spacing.md,
            theme.typography.title.size,
        ),
    }
}

/// Accessible button supporting 5 variants, 3 sizes, loading/disabled guards, focus ring,
/// and `Enter`/`Space` keyboard activation.
#[component]
pub fn Button(
    #[props(default)] variant: ButtonVariant,
    #[props(default)] size: ButtonSize,
    #[props(default)] loading: bool,
    #[props(default)] disabled: bool,
    #[props(default)] on_press: Option<EventHandler<()>>,
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] accessibility_hint: Option<String>,
    #[props(default)] aria_expanded: Option<String>,
    #[props(default)] aria_controls: Option<String>,
    #[props(default)] id: Option<String>,
    #[props(default)] button_type: Option<String>,
    #[props(default)] name: Option<String>,
    #[props(default)] value: Option<String>,
    #[props(default)] command: Option<String>,
    #[props(default)] commandfor: Option<String>,
    #[props(default)] popovertarget: Option<String>,
    #[props(default)] popovertargetaction: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut focused = use_signal(|| false);
    let blocked = disabled || loading;

    let ControlColors { bg, fg, border_width: bw, border: bc } =
        button_colors(&theme, variant, focused(), blocked);
    let (min_h, pad_x, pad_y, font_size) = button_metrics(&theme, size);

    let btn_style = TypedStyleBuilder::new()
        .flex_row()
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .gap(theme.spacing.xs)
        .min_height_px(min_h)
        .padding_xy(pad_x, pad_y)
        .surface(TypedSurfaceStyle {
            background: bg,
            border_width: bw,
            border_color: bc,
            border_radius: theme.radius.md,
            opacity: if blocked { 0.5 } else { 1.0 },
            box_shadow: "none",
        })
        .finish();

    let text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: fg,
            font_size,
            font_weight: "600",
            text_align: "center",
            opacity: 1.0,
        })
        .finish();

    let tab_index = if blocked { -1 } else { 0 };
    let on_press_click = on_press;
    let on_press_key = on_press;
    let html_attr = super::html_tables::html_invoker_table(
        button_type.clone(),
        name.clone(),
        value.clone(),
        command,
        commandfor,
        popovertarget,
        popovertargetaction,
    );

    rsx! {
        pressable {
            id: id,
            test_id: test_id,
            nve_table: html_attr,
            accessibility_role: "button",
            accessibility_label: accessibility_label,
            accessibility_hint: accessibility_hint,
            aria_expanded: aria_expanded,
            aria_controls: aria_controls,
            disabled: blocked,
            tab_index: tab_index,
            nve_style: btn_style,
            style: style,
            onfocus: move |_| focused.set(true),
            onblur: move |_| focused.set(false),
            onpress: move |_| {
                if !blocked && let Some(handler) = on_press_click {
                    handler.call(());
                }
            },
            onkeydown: move |ev: KeyEvent| {
                let data = ev.data();
                let key = data.key.as_str();
                if !blocked && matches!(key, "Enter" | " " | "Space")
                    && let Some(handler) = on_press_key
                {
                    handler.call(());
                }
            },
            if loading {
                text {
                    nve_style: text_style.clone(),
                    "..."
                }
            }
            text {
                nve_style: text_style,
                {children}
            }
        }
    }
}

/// Square icon-only button that **requires** `accessibility_label` for screen-reader clarity.
#[component]
pub fn IconButton(
    accessibility_label: String,
    #[props(default)] icon: String,
    #[props(default = ButtonVariant::Ghost)] variant: ButtonVariant,
    #[props(default)] size: ButtonSize,
    #[props(default)] loading: bool,
    #[props(default)] disabled: bool,
    #[props(default)] on_press: Option<EventHandler<()>>,
    #[props(default)] accessibility_hint: Option<String>,
    #[props(default)] aria_expanded: Option<String>,
    #[props(default)] aria_controls: Option<String>,
    #[props(default)] id: Option<String>,
    #[props(default)] button_type: Option<String>,
    #[props(default)] name: Option<String>,
    #[props(default)] value: Option<String>,
    #[props(default)] command: Option<String>,
    #[props(default)] commandfor: Option<String>,
    #[props(default)] popovertarget: Option<String>,
    #[props(default)] popovertargetaction: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    #[props(default)] children: Option<Element>,
) -> Element {
    let theme = use_theme();
    let mut focused = use_signal(|| false);
    let blocked = disabled || loading;

    let ControlColors { bg, fg, border_width: bw, border: bc } =
        button_colors(&theme, variant, focused(), blocked);
    let dim = match size {
        ButtonSize::Sm => 28.0,
        ButtonSize::Md => 36.0,
        ButtonSize::Lg => 44.0,
    };
    let font_size = match size {
        ButtonSize::Sm => theme.typography.ui.size,
        ButtonSize::Md => theme.typography.body.size,
        ButtonSize::Lg => theme.typography.title.size,
    };

    let btn_style = TypedStyleBuilder::new()
        .width_px(dim)
        .height_px(dim)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .surface(TypedSurfaceStyle {
            background: bg,
            border_width: bw,
            border_color: bc,
            border_radius: theme.radius.md,
            opacity: if blocked { 0.5 } else { 1.0 },
            box_shadow: "none",
        })
        .finish();

    let text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: fg,
            font_size,
            font_weight: "600",
            text_align: "center",
            opacity: 1.0,
        })
        .finish();

    let tab_index = if blocked { -1 } else { 0 };
    let on_press_click = on_press;
    let on_press_key = on_press;
    let html_attr = super::html_tables::html_invoker_table(
        button_type.clone(),
        name.clone(),
        value.clone(),
        command,
        commandfor,
        popovertarget,
        popovertargetaction,
    );

    rsx! {
        pressable {
            id: id,
            test_id: test_id,
            nve_table: html_attr,
            accessibility_role: "button",
            accessibility_label: "{accessibility_label}",
            accessibility_hint: accessibility_hint,
            aria_expanded: aria_expanded,
            aria_controls: aria_controls,
            disabled: blocked,
            tab_index: tab_index,
            nve_style: btn_style,
            style: style,
            onfocus: move |_| focused.set(true),
            onblur: move |_| focused.set(false),
            onpress: move |_| {
                if !blocked && let Some(handler) = on_press_click {
                    handler.call(());
                }
            },
            onkeydown: move |ev: KeyEvent| {
                let data = ev.data();
                let key = data.key.as_str();
                if !blocked && matches!(key, "Enter" | " " | "Space")
                    && let Some(handler) = on_press_key
                {
                    handler.call(());
                }
            },
            if loading {
                text { nve_style: text_style, "..." }
            } else if let Some(child) = children {
                {child}
            } else {
                text { nve_style: text_style, "{icon}" }
            }
        }
    }
}

/// Compact semantic pill badge (`Neutral`, `Accent`, `Success`, `Warning`, `Danger`).
#[component]
pub fn Badge(
    label: String,
    #[props(default)] variant: BadgeVariant,
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let theme = use_theme();
    let ControlColors { bg, fg, border: border_color, .. } = badge_colors(&theme, variant);

    let pill_style = TypedStyleBuilder::new()
        .flex_row()
        .align_items(FlexAlign::Center)
        .justify_content(FlexJustify::Center)
        .padding_xy(theme.spacing.sm, theme.spacing.xs)
        .surface(TypedSurfaceStyle {
            background: bg,
            border_width: 1.0,
            border_color,
            border_radius: theme.radius.full,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: fg,
            font_size: theme.typography.caption.size,
            font_weight: "600",
            text_align: "center",
            opacity: 1.0,
        })
        .finish();

    rsx! {
        view {
            test_id: test_id,
            accessibility_role: "status",
            accessibility_label: accessibility_label,
            nve_style: pill_style,
            style: style,
            text {
                nve_style: text_style,
                "{label}"
            }
        }
    }
}
