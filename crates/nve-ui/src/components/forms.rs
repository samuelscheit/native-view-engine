//! Accessible form controls: [`Input`], [`Checkbox`], and [`Switch`].

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::{ColorRgba, FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;

use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};
use crate::theme::use_theme_rgba;

/// Accessible text input with optional label, helper/error text (`aria-describedby`),
/// `secure`, `multiline`, `read_only`, `disabled`, and explicit focus/error ring styling.
#[component]
pub fn Input(
    #[props(default)] value: String,
    #[props(default)] on_change: Option<EventHandler<String>>,
    #[props(default)] on_submit: Option<EventHandler<String>>,
    #[props(default)] label: Option<String>,
    #[props(default)] placeholder: Option<String>,
    #[props(default)] helper_text: Option<String>,
    #[props(default)] error: Option<String>,
    #[props(default)] secure: bool,
    #[props(default)] multiline: bool,
    #[props(default)] read_only: bool,
    #[props(default)] disabled: bool,
    #[props(default)] id: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let (theme, colors) = use_theme_rgba();
    let mut focused = use_signal(|| false);

    let base_id = id
        .as_deref()
        .or(test_id.as_deref())
        .unwrap_or("nve-input");
    let label_id = label.as_ref().map(|_| format!("{base_id}-label"));
    let described_by = if error.is_some() {
        Some(format!("{base_id}-error"))
    } else if helper_text.is_some() {
        Some(format!("{base_id}-helper"))
    } else {
        None
    };

    let has_error = error.is_some();
    let (border_width, border_color) = if has_error {
        (2.0, colors.danger)
    } else if focused() && !disabled {
        (2.0, colors.border_focus)
    } else {
        (
            if theme.is_high_contrast() { 2.0 } else { 1.0 },
            colors.border,
        )
    };
    let field_bg = if disabled || read_only {
        colors.surface_subtle
    } else {
        colors.surface
    };

    let wrapper_style = TypedStyleBuilder::new()
        .flex_col()
        .gap(theme.spacing.xs)
        .width_pct(100.0)
        .finish();

    let input_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .min_height_px(if multiline { 88.0 } else { 38.0 })
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .surface(TypedSurfaceStyle {
            background: field_bg,
            border_width,
            border_color,
            border_radius: theme.radii.md,
            opacity: if disabled { 0.55 } else { 1.0 },
            box_shadow: "none",
        })
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.md,
            font_weight: "400",
            text_align: "left",
            opacity: if disabled { 0.55 } else { 1.0 },
        })
        .finish();

    let label_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.sm,
            font_weight: "600",
            text_align: "left",
            opacity: if disabled { 0.6 } else { 1.0 },
        })
        .finish();

    let desc_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: if has_error {
                colors.danger
            } else {
                colors.text_muted
            },
            font_size: theme.typography.xs,
            font_weight: if has_error { "500" } else { "400" },
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let tab_index = if disabled { -1 } else { 0 };
    let a11y_label = accessibility_label.or_else(|| label.clone());

    rsx! {
        view {
            nve_style: wrapper_style,
            style: style,
            if let Some(ref lbl) = label {
                text {
                    test_id: label_id.clone(),
                    nve_style: label_style,
                    "{lbl}"
                }
            }
            textinput {
                test_id: test_id,
                value: "{value}",
                placeholder: placeholder,
                multiline: multiline,
                secure_text_entry: secure,
                read_only: read_only,
                disabled: disabled,
                tab_index: tab_index,
                accessibility_role: "textbox",
                accessibility_label: a11y_label,
                aria_labelledby: label_id,
                aria_describedby: described_by.clone(),
                nve_style: input_style,
                onfocus: move |_| focused.set(true),
                onblur: move |_| focused.set(false),
                onchange: move |ev: TextEvent| {
                    if !disabled && !read_only && let Some(handler) = on_change {
                        handler.call(ev.data().value.clone());
                    }
                },
                onsubmit: move |ev: SubmitEvent| {
                    if !disabled && !read_only && let Some(handler) = on_submit {
                        handler.call(ev.data().value.clone());
                    }
                },
            }
            if let Some(ref err_msg) = error {
                text {
                    test_id: described_by.clone(),
                    accessibility_role: "alert",
                    aria_live: "assertive",
                    nve_style: desc_style,
                    "{err_msg}"
                }
            } else if let Some(ref help_msg) = helper_text {
                text {
                    test_id: described_by.clone(),
                    nve_style: desc_style,
                    "{help_msg}"
                }
            }
        }
    }
}

/// Accessible tri-state checkbox (`checked`, `indeterminate`, `aria-checked`, `Space` toggle).
#[component]
pub fn Checkbox(
    #[props(default)] checked: bool,
    #[props(default)] indeterminate: bool,
    #[props(default)] disabled: bool,
    #[props(default)] label: Option<String>,
    #[props(default)] on_change: Option<EventHandler<bool>>,
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let (theme, colors) = use_theme_rgba();
    let mut focused = use_signal(|| false);

    let aria_checked = if indeterminate {
        "mixed"
    } else if checked {
        "true"
    } else {
        "false"
    };
    let active_fill = checked || indeterminate;

    let row_style = TypedStyleBuilder::new()
        .flex_row()
        .align_items(FlexAlign::Center)
        .gap(theme.spacing.sm)
        .surface(TypedSurfaceStyle {
            background: ColorRgba::TRANSPARENT,
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: theme.radii.sm,
            opacity: if disabled { 0.5 } else { 1.0 },
            box_shadow: "none",
        })
        .finish();

    let (box_bg, box_bw, box_bc) = if focused() && !disabled {
        (
            if active_fill {
                colors.accent
            } else {
                colors.surface
            },
            2.0,
            colors.border_focus,
        )
    } else if active_fill {
        (colors.accent, 1.0, colors.accent)
    } else {
        (
            colors.surface,
            if theme.is_high_contrast() { 2.0 } else { 1.0 },
            colors.border,
        )
    };

    let box_style = TypedStyleBuilder::new()
        .width_px(20.0)
        .height_px(20.0)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .surface(TypedSurfaceStyle {
            background: box_bg,
            border_width: box_bw,
            border_color: box_bc,
            border_radius: theme.radii.sm,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let mark_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.accent_fg,
            font_size: theme.typography.sm,
            font_weight: "700",
            text_align: "center",
            opacity: 1.0,
        })
        .finish();

    let lbl_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.md,
            font_weight: "400",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let glyph = if indeterminate {
        "-"
    } else if checked {
        "x"
    } else {
        ""
    };
    let tab_index = if disabled { -1 } else { 0 };
    let a11y_label = accessibility_label.or_else(|| label.clone());

    rsx! {
        pressable {
            test_id: test_id,
            accessibility_role: "checkbox",
            accessibility_label: a11y_label,
            aria_checked: "{aria_checked}",
            disabled: disabled,
            tab_index: tab_index,
            nve_style: row_style,
            style: style,
            onfocus: move |_| focused.set(true),
            onblur: move |_| focused.set(false),
            onpress: move |_| {
                if !disabled && let Some(handler) = on_change {
                    handler.call(!checked);
                }
            },
            onkeydown: move |ev: KeyEvent| {
                let data = ev.data();
                let key = data.key.as_str();
                if !disabled && matches!(key, " " | "Space") && let Some(handler) = on_change {
                    handler.call(!checked);
                }
            },
            view {
                nve_style: box_style,
                text {
                    nve_style: mark_style,
                    "{glyph}"
                }
            }
            if let Some(ref lbl) = label {
                text {
                    nve_style: lbl_style,
                    "{lbl}"
                }
            }
        }
    }
}

/// Accessible toggle switch (`accessibilityRole="switch"`, `aria-checked`, `Space`/`Enter` toggle).
#[component]
pub fn Switch(
    #[props(default)] checked: bool,
    #[props(default)] disabled: bool,
    #[props(default)] label: Option<String>,
    #[props(default)] on_change: Option<EventHandler<bool>>,
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let (theme, colors) = use_theme_rgba();
    let mut focused = use_signal(|| false);

    let aria_checked = if checked { "true" } else { "false" };

    let row_style = TypedStyleBuilder::new()
        .flex_row()
        .align_items(FlexAlign::Center)
        .gap(theme.spacing.sm)
        .surface(TypedSurfaceStyle {
            background: ColorRgba::TRANSPARENT,
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: theme.radii.sm,
            opacity: if disabled { 0.5 } else { 1.0 },
            box_shadow: "none",
        })
        .finish();

    let (track_bg, track_bw, track_bc) = if focused() && !disabled {
        (
            if checked { colors.accent } else { colors.border },
            2.0,
            colors.border_focus,
        )
    } else if checked {
        (colors.accent, 1.0, colors.accent)
    } else {
        (colors.border, 1.0, colors.border)
    };

    let track_style = TypedStyleBuilder::new()
        .width_px(44.0)
        .height_px(24.0)
        .padding(3.0)
        .flex_row()
        .align_items(FlexAlign::Center)
        .justify_content(if checked {
            FlexJustify::FlexEnd
        } else {
            FlexJustify::FlexStart
        })
        .surface(TypedSurfaceStyle {
            background: track_bg,
            border_width: track_bw,
            border_color: track_bc,
            border_radius: theme.radii.full,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let thumb_style = TypedStyleBuilder::new()
        .width_px(18.0)
        .height_px(18.0)
        .surface(TypedSurfaceStyle {
            background: if checked {
                colors.accent_fg
            } else {
                colors.surface
            },
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: theme.radii.full,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let lbl_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.md,
            font_weight: "400",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let tab_index = if disabled { -1 } else { 0 };
    let a11y_label = accessibility_label.or_else(|| label.clone());

    rsx! {
        pressable {
            test_id: test_id,
            accessibility_role: "switch",
            accessibility_label: a11y_label,
            aria_checked: "{aria_checked}",
            disabled: disabled,
            tab_index: tab_index,
            nve_style: row_style,
            style: style,
            onfocus: move |_| focused.set(true),
            onblur: move |_| focused.set(false),
            onpress: move |_| {
                if !disabled && let Some(handler) = on_change {
                    handler.call(!checked);
                }
            },
            onkeydown: move |ev: KeyEvent| {
                let data = ev.data();
                let key = data.key.as_str();
                if !disabled && matches!(key, " " | "Space" | "Enter")
                    && let Some(handler) = on_change
                {
                    handler.call(!checked);
                }
            },
            view {
                nve_style: track_style,
                view {
                    nve_style: thumb_style,
                }
            }
            if let Some(ref lbl) = label {
                text {
                    nve_style: lbl_style,
                    "{lbl}"
                }
            }
        }
    }
}
