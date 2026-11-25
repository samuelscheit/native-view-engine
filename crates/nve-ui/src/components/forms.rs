//! Accessible form controls: [`Input`], [`Checkbox`], and [`Switch`].

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;

use crate::style_buf::{StyleBuf, SurfaceStyle, TextStyle};
use crate::theme::use_theme;

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
    let theme = use_theme();
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
        (2.0, theme.palette.danger.as_ref())
    } else if focused() && !disabled {
        (2.0, theme.palette.border_focus.as_ref())
    } else {
        (
            if theme.is_high_contrast() { 2.0 } else { 1.0 },
            theme.palette.border.as_ref(),
        )
    };
    let field_bg = if disabled || read_only {
        theme.palette.surface_subtle.as_ref()
    } else {
        theme.palette.surface.as_ref()
    };

    let mut wrapper_sb = StyleBuf::new();
    wrapper_sb
        .flex_direction("column")
        .gap(theme.spacing.xs)
        .width_pct(100.0);
    if let Some(ref extra) = style {
        wrapper_sb.raw(extra);
    }
    let wrapper_style = wrapper_sb.finish();

    let mut input_sb = StyleBuf::new();
    input_sb
        .width_pct(100.0)
        .min_height_px(if multiline { 88.0 } else { 38.0 })
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .surface(SurfaceStyle {
            background: field_bg,
            border_width,
            border_color,
            border_radius: theme.radii.md,
            opacity: if disabled { 0.55 } else { 1.0 },
            box_shadow: "none",
        })
        .text_style(TextStyle {
            color: theme.palette.text_primary.as_ref(),
            font_size: theme.typography.md,
            font_weight: "400",
            text_align: "left",
            opacity: if disabled { 0.55 } else { 1.0 },
        });
    let input_style = input_sb.finish();

    let mut label_sb = StyleBuf::new();
    label_sb.text_style(TextStyle {
        color: theme.palette.text_primary.as_ref(),
        font_size: theme.typography.sm,
        font_weight: "600",
        text_align: "left",
        opacity: if disabled { 0.6 } else { 1.0 },
    });
    let label_style = label_sb.finish();

    let mut desc_sb = StyleBuf::new();
    desc_sb.text_style(TextStyle {
        color: if has_error {
            theme.palette.danger.as_ref()
        } else {
            theme.palette.text_muted.as_ref()
        },
        font_size: theme.typography.xs,
        font_weight: if has_error { "500" } else { "400" },
        text_align: "left",
        opacity: 1.0,
    });
    let desc_style = desc_sb.finish();

    let tab_index = if disabled { -1 } else { 0 };
    let a11y_label = accessibility_label.or_else(|| label.clone());

    rsx! {
        view {
            style: "{wrapper_style}",
            if let Some(ref lbl) = label {
                text {
                    test_id: label_id.clone(),
                    style: "{label_style}",
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
                style: "{input_style}",
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
                    style: "{desc_style}",
                    "{err_msg}"
                }
            } else if let Some(ref help_msg) = helper_text {
                text {
                    test_id: described_by.clone(),
                    style: "{desc_style}",
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
    let theme = use_theme();
    let mut focused = use_signal(|| false);

    let aria_checked = if indeterminate {
        "mixed"
    } else if checked {
        "true"
    } else {
        "false"
    };
    let active_fill = checked || indeterminate;

    let mut row_sb = StyleBuf::new();
    row_sb
        .flex_direction("row")
        .align_items("center")
        .gap(theme.spacing.sm)
        .surface(SurfaceStyle {
            background: "transparent",
            border_width: 0.0,
            border_color: "transparent",
            border_radius: theme.radii.sm,
            opacity: if disabled { 0.5 } else { 1.0 },
            box_shadow: "none",
        });
    if let Some(ref extra) = style {
        row_sb.raw(extra);
    }
    let row_style = row_sb.finish();

    let (box_bg, box_bw, box_bc) = if focused() && !disabled {
        (
            if active_fill {
                theme.palette.accent.as_ref()
            } else {
                theme.palette.surface.as_ref()
            },
            2.0,
            theme.palette.border_focus.as_ref(),
        )
    } else if active_fill {
        (
            theme.palette.accent.as_ref(),
            1.0,
            theme.palette.accent.as_ref(),
        )
    } else {
        (
            theme.palette.surface.as_ref(),
            if theme.is_high_contrast() { 2.0 } else { 1.0 },
            theme.palette.border.as_ref(),
        )
    };

    let mut box_sb = StyleBuf::new();
    box_sb
        .width_px(20.0)
        .height_px(20.0)
        .justify_content("center")
        .align_items("center")
        .surface(SurfaceStyle {
            background: box_bg,
            border_width: box_bw,
            border_color: box_bc,
            border_radius: theme.radii.sm,
            opacity: 1.0,
            box_shadow: "none",
        });
    let box_style = box_sb.finish();

    let mut mark_sb = StyleBuf::new();
    mark_sb.text_style(TextStyle {
        color: theme.palette.accent_fg.as_ref(),
        font_size: theme.typography.sm,
        font_weight: "700",
        text_align: "center",
        opacity: 1.0,
    });
    let mark_style = mark_sb.finish();

    let mut lbl_sb = StyleBuf::new();
    lbl_sb.text_style(TextStyle {
        color: theme.palette.text_primary.as_ref(),
        font_size: theme.typography.md,
        font_weight: "400",
        text_align: "left",
        opacity: 1.0,
    });
    let lbl_style = lbl_sb.finish();

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
            style: "{row_style}",
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
                style: "{box_style}",
                text {
                    style: "{mark_style}",
                    "{glyph}"
                }
            }
            if let Some(ref lbl) = label {
                text {
                    style: "{lbl_style}",
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
    let theme = use_theme();
    let mut focused = use_signal(|| false);

    let aria_checked = if checked { "true" } else { "false" };

    let mut row_sb = StyleBuf::new();
    row_sb
        .flex_direction("row")
        .align_items("center")
        .gap(theme.spacing.sm)
        .surface(SurfaceStyle {
            background: "transparent",
            border_width: 0.0,
            border_color: "transparent",
            border_radius: theme.radii.sm,
            opacity: if disabled { 0.5 } else { 1.0 },
            box_shadow: "none",
        });
    if let Some(ref extra) = style {
        row_sb.raw(extra);
    }
    let row_style = row_sb.finish();

    let (track_bg, track_bw, track_bc) = if focused() && !disabled {
        (
            if checked {
                theme.palette.accent.as_ref()
            } else {
                theme.palette.border.as_ref()
            },
            2.0,
            theme.palette.border_focus.as_ref(),
        )
    } else if checked {
        (
            theme.palette.accent.as_ref(),
            1.0,
            theme.palette.accent.as_ref(),
        )
    } else {
        (
            theme.palette.border.as_ref(),
            1.0,
            theme.palette.border.as_ref(),
        )
    };

    let mut track_sb = StyleBuf::new();
    track_sb
        .width_px(44.0)
        .height_px(24.0)
        .padding(3.0)
        .flex_direction("row")
        .align_items("center")
        .justify_content(if checked { "flex-end" } else { "flex-start" })
        .surface(SurfaceStyle {
            background: track_bg,
            border_width: track_bw,
            border_color: track_bc,
            border_radius: theme.radii.full,
            opacity: 1.0,
            box_shadow: "none",
        });
    let track_style = track_sb.finish();

    let mut thumb_sb = StyleBuf::new();
    thumb_sb
        .width_px(18.0)
        .height_px(18.0)
        .surface(SurfaceStyle {
            background: if checked {
                theme.palette.accent_fg.as_ref()
            } else {
                theme.palette.surface.as_ref()
            },
            border_width: 0.0,
            border_color: "transparent",
            border_radius: theme.radii.full,
            opacity: 1.0,
            box_shadow: "none",
        });
    let thumb_style = thumb_sb.finish();

    let mut lbl_sb = StyleBuf::new();
    lbl_sb.text_style(TextStyle {
        color: theme.palette.text_primary.as_ref(),
        font_size: theme.typography.md,
        font_weight: "400",
        text_align: "left",
        opacity: 1.0,
    });
    let lbl_style = lbl_sb.finish();

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
            style: "{row_style}",
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
                style: "{track_style}",
                view {
                    style: "{thumb_style}",
                }
            }
            if let Some(ref lbl) = label {
                text {
                    style: "{lbl_style}",
                    "{lbl}"
                }
            }
        }
    }
}
