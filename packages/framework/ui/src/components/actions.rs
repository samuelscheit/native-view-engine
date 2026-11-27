//! Action and display components: [`Button`], [`IconButton`], and [`Badge`].

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::{ColorRgba, FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;

use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};
use crate::theme::{use_theme_rgba, UiPaletteRgba, UiTheme};

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

fn button_colors(
    theme: &UiTheme,
    colors: &UiPaletteRgba,
    variant: ButtonVariant,
    focused: bool,
    disabled: bool,
) -> (ColorRgba, ColorRgba, f32, ColorRgba) {
    let (bg, fg, base_bw, base_bc) = match variant {
        ButtonVariant::Primary => (
            colors.accent,
            colors.accent_fg,
            if theme.is_high_contrast() { 2.0 } else { 0.0 },
            if theme.is_high_contrast() {
                colors.border
            } else {
                ColorRgba::TRANSPARENT
            },
        ),
        ButtonVariant::Secondary => (colors.surface_subtle, colors.text_primary, 1.0, colors.border),
        ButtonVariant::Ghost => (
            ColorRgba::TRANSPARENT,
            colors.text_primary,
            0.0,
            ColorRgba::TRANSPARENT,
        ),
        ButtonVariant::Destructive => (
            colors.danger,
            colors.danger_fg,
            if theme.is_high_contrast() { 2.0 } else { 0.0 },
            if theme.is_high_contrast() {
                colors.border
            } else {
                ColorRgba::TRANSPARENT
            },
        ),
        ButtonVariant::Outline => (
            colors.surface,
            colors.text_primary,
            if theme.is_high_contrast() { 2.0 } else { 1.0 },
            colors.border,
        ),
    };

    if focused && !disabled {
        (bg, fg, 2.0, colors.border_focus)
    } else {
        (bg, fg, base_bw, base_bc)
    }
}

fn button_metrics(theme: &UiTheme, size: ButtonSize) -> (f32, f32, f32, f32) {
    let pad_scale = match theme.density {
        crate::theme::UiDensity::Normal => 1.0,
        crate::theme::UiDensity::Compact => 0.8,
        crate::theme::UiDensity::Spacious => 1.15,
    };
    match size {
        ButtonSize::Sm => (
            30.0 * pad_scale,
            theme.spacing.sm,
            theme.spacing.xs,
            theme.typography.sm,
        ),
        ButtonSize::Md => (
            38.0 * pad_scale,
            theme.spacing.lg,
            theme.spacing.sm,
            theme.typography.md,
        ),
        ButtonSize::Lg => (
            46.0 * pad_scale,
            theme.spacing.xl,
            theme.spacing.md,
            theme.typography.lg,
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
    let (theme, colors) = use_theme_rgba();
    let mut focused = use_signal(|| false);
    let blocked = disabled || loading;

    let (bg, fg, bw, bc) = button_colors(&theme, &colors, variant, focused(), blocked);
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
            border_radius: theme.radii.md,
            opacity: if blocked { 0.5 } else { 1.0 },
            box_shadow: None,
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
    let (theme, colors) = use_theme_rgba();
    let mut focused = use_signal(|| false);
    let blocked = disabled || loading;

    let (bg, fg, bw, bc) = button_colors(&theme, &colors, variant, focused(), blocked);
    let dim = match size {
        ButtonSize::Sm => 28.0,
        ButtonSize::Md => 36.0,
        ButtonSize::Lg => 44.0,
    };
    let font_size = match size {
        ButtonSize::Sm => theme.typography.sm,
        ButtonSize::Md => theme.typography.md,
        ButtonSize::Lg => theme.typography.lg,
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
            border_radius: theme.radii.md,
            opacity: if blocked { 0.5 } else { 1.0 },
            box_shadow: None,
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
    let (theme, colors) = use_theme_rgba();
    let (bg, fg, border_color) = match variant {
        BadgeVariant::Neutral => (colors.surface_subtle, colors.text_secondary, colors.border),
        BadgeVariant::Accent => (colors.accent, colors.accent_fg, colors.accent),
        BadgeVariant::Success => (colors.success, colors.accent_fg, colors.success),
        BadgeVariant::Warning => (
            colors.warning,
            if theme.is_high_contrast() {
                ColorRgba::BLACK
            } else {
                ColorRgba::WHITE
            },
            colors.warning,
        ),
        BadgeVariant::Danger => (colors.danger, colors.danger_fg, colors.danger),
    };

    let pill_style = TypedStyleBuilder::new()
        .flex_row()
        .align_items(FlexAlign::Center)
        .justify_content(FlexJustify::Center)
        .padding_xy(theme.spacing.sm, theme.spacing.xs)
        .surface(TypedSurfaceStyle {
            background: bg,
            border_width: 1.0,
            border_color,
            border_radius: theme.radii.full,
            opacity: 1.0,
            box_shadow: None,
        })
        .finish();

    let text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: fg,
            font_size: theme.typography.xs,
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
