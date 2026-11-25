//! Action and display components: [`Button`], [`IconButton`], and [`Badge`].

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;

use crate::style_buf::{StyleBuf, SurfaceStyle, TextStyle};
use crate::theme::{use_theme, UiTheme};

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
    variant: ButtonVariant,
    focused: bool,
    disabled: bool,
) -> (&str, &str, f32, &str) {
    let (bg, fg, base_bw, base_bc) = match variant {
        ButtonVariant::Primary => (
            theme.palette.accent.as_ref(),
            theme.palette.accent_fg.as_ref(),
            if theme.is_high_contrast() { 2.0 } else { 0.0 },
            if theme.is_high_contrast() {
                theme.palette.border.as_ref()
            } else {
                "transparent"
            },
        ),
        ButtonVariant::Secondary => (
            theme.palette.surface_subtle.as_ref(),
            theme.palette.text_primary.as_ref(),
            1.0,
            theme.palette.border.as_ref(),
        ),
        ButtonVariant::Ghost => (
            "transparent",
            theme.palette.text_primary.as_ref(),
            0.0,
            "transparent",
        ),
        ButtonVariant::Destructive => (
            theme.palette.danger.as_ref(),
            theme.palette.danger_fg.as_ref(),
            if theme.is_high_contrast() { 2.0 } else { 0.0 },
            if theme.is_high_contrast() {
                theme.palette.border.as_ref()
            } else {
                "transparent"
            },
        ),
        ButtonVariant::Outline => (
            theme.palette.surface.as_ref(),
            theme.palette.text_primary.as_ref(),
            if theme.is_high_contrast() { 2.0 } else { 1.0 },
            theme.palette.border.as_ref(),
        ),
    };

    if focused && !disabled {
        (bg, fg, 2.0, theme.palette.border_focus.as_ref())
    } else {
        (bg, fg, base_bw, base_bc)
    }
}

fn button_metrics(theme: &UiTheme, size: ButtonSize) -> (f32, f32, f32, f32) {
    let pad_scale = match theme.density {
        crate::theme::UiDensity::Normal => 1.0,
        crate::theme::UiDensity::Compact => 0.8,
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
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut focused = use_signal(|| false);
    let blocked = disabled || loading;

    let (bg, fg, bw, bc) = button_colors(&theme, variant, focused(), blocked);
    let (min_h, pad_x, pad_y, font_size) = button_metrics(&theme, size);

    let mut sb = StyleBuf::new();
    sb.flex_direction("row")
        .justify_content("center")
        .align_items("center")
        .gap(theme.spacing.xs)
        .min_height_px(min_h)
        .padding_xy(pad_x, pad_y)
        .surface(SurfaceStyle {
            background: bg,
            border_width: bw,
            border_color: bc,
            border_radius: theme.radii.md,
            opacity: if blocked { 0.5 } else { 1.0 },
            box_shadow: "none",
        });
    if let Some(ref extra) = style {
        sb.raw(extra);
    }
    let btn_style = sb.finish();

    let mut ts = StyleBuf::new();
    ts.text_style(TextStyle {
        color: fg,
        font_size,
        font_weight: "600",
        text_align: "center",
        opacity: 1.0,
    });
    let text_style = ts.finish();

    let tab_index = if blocked { -1 } else { 0 };
    let on_press_click = on_press;
    let on_press_key = on_press;

    rsx! {
        pressable {
            test_id: test_id,
            accessibility_role: "button",
            accessibility_label: accessibility_label,
            accessibility_hint: accessibility_hint,
            aria_expanded: aria_expanded,
            aria_controls: aria_controls,
            disabled: blocked,
            tab_index: tab_index,
            style: "{btn_style}",
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
                    style: "{text_style}",
                    "..."
                }
            }
            text {
                style: "{text_style}",
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
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    #[props(default)] children: Option<Element>,
) -> Element {
    let theme = use_theme();
    let mut focused = use_signal(|| false);
    let blocked = disabled || loading;

    let (bg, fg, bw, bc) = button_colors(&theme, variant, focused(), blocked);
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

    let mut sb = StyleBuf::new();
    sb.width_px(dim)
        .height_px(dim)
        .justify_content("center")
        .align_items("center")
        .surface(SurfaceStyle {
            background: bg,
            border_width: bw,
            border_color: bc,
            border_radius: theme.radii.md,
            opacity: if blocked { 0.5 } else { 1.0 },
            box_shadow: "none",
        });
    if let Some(ref extra) = style {
        sb.raw(extra);
    }
    let btn_style = sb.finish();

    let mut ts = StyleBuf::new();
    ts.text_style(TextStyle {
        color: fg,
        font_size,
        font_weight: "600",
        text_align: "center",
        opacity: 1.0,
    });
    let text_style = ts.finish();

    let tab_index = if blocked { -1 } else { 0 };
    let on_press_click = on_press;
    let on_press_key = on_press;

    rsx! {
        pressable {
            test_id: test_id,
            accessibility_role: "button",
            accessibility_label: "{accessibility_label}",
            accessibility_hint: accessibility_hint,
            aria_expanded: aria_expanded,
            aria_controls: aria_controls,
            disabled: blocked,
            tab_index: tab_index,
            style: "{btn_style}",
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
                text { style: "{text_style}", "..." }
            } else if let Some(child) = children {
                {child}
            } else {
                text { style: "{text_style}", "{icon}" }
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
    let (bg, fg, border_color) = match variant {
        BadgeVariant::Neutral => (
            theme.palette.surface_subtle.as_ref(),
            theme.palette.text_secondary.as_ref(),
            theme.palette.border.as_ref(),
        ),
        BadgeVariant::Accent => (
            theme.palette.accent.as_ref(),
            theme.palette.accent_fg.as_ref(),
            theme.palette.accent.as_ref(),
        ),
        BadgeVariant::Success => (
            theme.palette.success.as_ref(),
            theme.palette.accent_fg.as_ref(),
            theme.palette.success.as_ref(),
        ),
        BadgeVariant::Warning => (
            theme.palette.warning.as_ref(),
            if theme.is_high_contrast() {
                "#000000"
            } else {
                "#ffffff"
            },
            theme.palette.warning.as_ref(),
        ),
        BadgeVariant::Danger => (
            theme.palette.danger.as_ref(),
            theme.palette.danger_fg.as_ref(),
            theme.palette.danger.as_ref(),
        ),
    };

    let mut sb = StyleBuf::new();
    sb.flex_direction("row")
        .align_items("center")
        .justify_content("center")
        .padding_xy(theme.spacing.sm, theme.spacing.xs)
        .surface(SurfaceStyle {
            background: bg,
            border_width: 1.0,
            border_color,
            border_radius: theme.radii.full,
            opacity: 1.0,
            box_shadow: "none",
        });
    if let Some(ref extra) = style {
        sb.raw(extra);
    }
    let pill_style = sb.finish();

    let mut ts = StyleBuf::new();
    ts.text_style(TextStyle {
        color: fg,
        font_size: theme.typography.xs,
        font_weight: "600",
        text_align: "center",
        opacity: 1.0,
    });
    let text_style = ts.finish();

    rsx! {
        view {
            test_id: test_id,
            accessibility_role: "status",
            accessibility_label: accessibility_label,
            style: "{pill_style}",
            text {
                style: "{text_style}",
                "{label}"
            }
        }
    }
}
