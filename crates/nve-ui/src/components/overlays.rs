//! Accessible overlay components: [`Dialog`] / [`Modal`], [`Popover`], [`Tooltip`],
//! and re-exported [`DropdownMenu`] / [`MenuItem`].

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;

pub use crate::components::dropdown::{DropdownMenu, MenuItem};
use crate::focus::{
    current_focused_id, notify_focus_scope, FocusTrap, FocusTrapAction, FocusableEntry,
};
use crate::style_buf::{StyleBuf, SurfaceStyle, TextStyle};
use crate::theme::use_theme;

/// Accessible modal dialog with `accessibilityRole="dialog"`, `ariaModal="true"`,
/// [`FocusTrap`] (`Tab`/`Shift+Tab` wrap-around + focus restoration), `Escape` dismissal,
/// and optional backdrop click dismissal.
#[component]
pub fn Dialog(
    open: bool,
    #[props(default)] title: Option<String>,
    #[props(default)] description: Option<String>,
    #[props(default)] focusable_ids: Vec<String>,
    #[props(default)] previous_focus: Option<String>,
    #[props(default = true)] close_on_escape: bool,
    #[props(default = true)] close_on_backdrop: bool,
    #[props(default)] on_close: Option<EventHandler<()>>,
    #[props(default)] on_focus_change: Option<EventHandler<Option<String>>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut trap = use_signal(|| {
        FocusTrap::from_ids(focusable_ids.iter().cloned()).with_close_on_escape(close_on_escape)
    });
    let mut was_open = use_signal(|| false);

    // Sync focusable items and open/close lifecycle without triggering spurious render loops.
    {
        let mut t = trap.write_unchecked();
        t.scope_mut().set_items(
            focusable_ids
                .iter()
                .cloned()
                .map(FocusableEntry::new)
                .collect(),
        );
    }

    if open && !*was_open.peek() {
        *was_open.write_unchecked() = true;
        let prev = previous_focus.clone().or_else(current_focused_id);
        let initial_focus = {
            let mut t = trap.write_unchecked();
            t.activate(prev).map(str::to_owned)
        };
        if let Some(ref fid) = initial_focus {
            notify_focus_scope(fid);
        }
        if let Some(handler) = on_focus_change {
            handler.call(initial_focus);
        }
    } else if !open && *was_open.peek() {
        *was_open.write_unchecked() = false;
        let restored = {
            let mut t = trap.write_unchecked();
            t.deactivate()
        };
        if let Some(ref fid) = restored {
            notify_focus_scope(fid);
        }
        if let Some(handler) = on_focus_change {
            handler.call(restored);
        }
    }

    if !open {
        return rsx! {};
    }

    let base_id = test_id.as_deref().unwrap_or("nve-dialog");
    let backdrop_id = format!("{base_id}-backdrop");
    let title_id = title.as_ref().map(|_| format!("{base_id}-title"));
    let desc_id = description.as_ref().map(|_| format!("{base_id}-description"));

    let mut backdrop_sb = StyleBuf::new();
    backdrop_sb
        .width_pct(100.0)
        .height_pct(100.0)
        .justify_content("center")
        .align_items("center")
        .padding(theme.spacing.xl)
        .z_index(1000)
        .surface(SurfaceStyle {
            background: theme.backdrop_scrim(),
            border_width: 0.0,
            border_color: "transparent",
            border_radius: 0.0,
            opacity: 1.0,
            box_shadow: "none",
        });
    let backdrop_style = backdrop_sb.finish();

    let mut panel_sb = StyleBuf::new();
    panel_sb
        .flex_direction("column")
        .min_width_px(320.0)
        .max_width_px(560.0)
        .padding(theme.spacing.xl)
        .gap(theme.spacing.md)
        .z_index(1001)
        .surface(SurfaceStyle {
            background: theme.palette.surface_elevated.as_ref(),
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: theme.palette.border.as_ref(),
            border_radius: theme.radii.lg,
            opacity: 1.0,
            box_shadow: theme.elevation_shadow(),
        });
    if let Some(ref extra) = style {
        panel_sb.raw(extra);
    }
    let panel_style = panel_sb.finish();

    let mut title_sb = StyleBuf::new();
    title_sb.text_style(TextStyle {
        color: theme.palette.text_primary.as_ref(),
        font_size: theme.typography.lg,
        font_weight: "700",
        text_align: "left",
        opacity: 1.0,
    });
    let title_style = title_sb.finish();

    let mut desc_sb = StyleBuf::new();
    desc_sb.text_style(TextStyle {
        color: theme.palette.text_secondary.as_ref(),
        font_size: theme.typography.sm,
        font_weight: "400",
        text_align: "left",
        opacity: 1.0,
    });
    let desc_style = desc_sb.finish();

    let active_focus = trap.read().active().map(str::to_owned);

    rsx! {
        pressable {
            test_id: "{backdrop_id}",
            accessibility_role: "presentation",
            style: "{backdrop_style}",
            onpress: move |_| {
                if close_on_backdrop {
                    let restored = trap.write().deactivate();
                    was_open.set(false);
                    if let Some(ref fid) = restored {
                        notify_focus_scope(fid);
                    }
                    if let Some(handler) = on_focus_change {
                        handler.call(restored);
                    }
                    if let Some(handler) = on_close {
                        handler.call(());
                    }
                }
            },
            view {
                test_id: "{base_id}",
                accessibility_role: "dialog",
                aria_modal: "true",
                aria_labelledby: title_id.clone(),
                aria_describedby: desc_id.clone(),
                aria_controls: active_focus,
                tab_index: -1,
                style: "{panel_style}",
                onkeydown: move |ev: KeyEvent| {
                    let data = ev.data();
                    let action = trap.write().handle_key(&data.key, data.shift);
                    match action {
                        FocusTrapAction::FocusChanged(next_id) => {
                            notify_focus_scope(&next_id);
                            if let Some(handler) = on_focus_change {
                                handler.call(Some(next_id));
                            }
                        }
                        FocusTrapAction::Dismiss { restore_focus } => {
                            was_open.set(false);
                            if let Some(ref fid) = restore_focus {
                                notify_focus_scope(fid);
                            }
                            if let Some(handler) = on_focus_change {
                                handler.call(restore_focus);
                            }
                            if let Some(handler) = on_close {
                                handler.call(());
                            }
                        }
                        FocusTrapAction::Ignored => {}
                    }
                },
                if let Some(ref heading) = title {
                    text {
                        test_id: title_id.clone(),
                        style: "{title_style}",
                        "{heading}"
                    }
                }
                if let Some(ref desc) = description {
                    text {
                        test_id: desc_id.clone(),
                        style: "{desc_style}",
                        "{desc}"
                    }
                }
                {children}
            }
        }
    }
}

/// Alias for [`Dialog`] supporting modal dialog ergonomics (`Modal { open: ... }`).
pub use Dialog as Modal;
pub use DialogProps as ModalProps;

/// Accessible anchored popover (`aria-expanded`, `aria-controls`, `Escape` dismiss).
#[component]
pub fn Popover(
    open: bool,
    trigger_label: String,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default = true)] close_on_escape: bool,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut focused = use_signal(|| false);

    let base_id = test_id.as_deref().unwrap_or("nve-popover");
    let trigger_id = format!("{base_id}-trigger");
    let content_id = format!("{base_id}-content");

    let mut wrap_sb = StyleBuf::new();
    wrap_sb.flex_direction("column").gap(theme.spacing.xs);
    if let Some(ref extra) = style {
        wrap_sb.raw(extra);
    }
    let wrap_style = wrap_sb.finish();

    let mut trigger_sb = StyleBuf::new();
    trigger_sb
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .justify_content("center")
        .align_items("center")
        .surface(SurfaceStyle {
            background: theme.palette.surface.as_ref(),
            border_width: if focused() { 2.0 } else { 1.0 },
            border_color: if focused() {
                theme.palette.border_focus.as_ref()
            } else {
                theme.palette.border.as_ref()
            },
            border_radius: theme.radii.md,
            opacity: 1.0,
            box_shadow: "none",
        });
    let trigger_style = trigger_sb.finish();

    let mut trigger_ts = StyleBuf::new();
    trigger_ts.text_style(TextStyle {
        color: theme.palette.text_primary.as_ref(),
        font_size: theme.typography.md,
        font_weight: "500",
        text_align: "center",
        opacity: 1.0,
    });
    let trigger_text_style = trigger_ts.finish();

    let mut panel_sb = StyleBuf::new();
    panel_sb
        .flex_direction("column")
        .padding(theme.spacing.md)
        .gap(theme.spacing.sm)
        .z_index(900)
        .surface(SurfaceStyle {
            background: theme.palette.surface_elevated.as_ref(),
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: theme.palette.border.as_ref(),
            border_radius: theme.radii.md,
            opacity: 1.0,
            box_shadow: theme.elevation_shadow(),
        });
    let panel_style = panel_sb.finish();

    let aria_expanded = if open { "true" } else { "false" };

    rsx! {
        view {
            test_id: "{base_id}",
            style: "{wrap_style}",
            pressable {
                test_id: "{trigger_id}",
                accessibility_role: "button",
                aria_expanded: "{aria_expanded}",
                aria_controls: "{content_id}",
                tab_index: 0,
                style: "{trigger_style}",
                onfocus: move |_| focused.set(true),
                onblur: move |_| focused.set(false),
                onpress: move |_| {
                    if let Some(handler) = on_open_change {
                        handler.call(!open);
                    }
                },
                onkeydown: move |ev: KeyEvent| {
                    let data = ev.data();
                    let key = data.key.as_str();
                    if let Some(handler) = on_open_change {
                        match key {
                            "Escape" | "Esc" if open && close_on_escape => handler.call(false),
                            "Enter" | " " | "Space" => handler.call(!open),
                            _ => {}
                        }
                    }
                },
                text {
                    style: "{trigger_text_style}",
                    "{trigger_label}"
                }
            }
            if open {
                view {
                    test_id: "{content_id}",
                    accessibility_role: "dialog",
                    tab_index: -1,
                    style: "{panel_style}",
                    onkeydown: move |ev: KeyEvent| {
                        let data = ev.data();
                        let key = data.key.as_str();
                        if close_on_escape && matches!(key, "Escape" | "Esc")
                            && let Some(handler) = on_open_change
                        {
                            handler.call(false);
                        }
                    },
                    {children}
                }
            }
        }
    }
}

/// Accessible tooltip (`accessibilityRole="tooltip"`, `aria-expanded`, `aria-describedby`,
/// `Escape` dismiss). Works in both controlled (`open: Some(bool)`) and uncontrolled modes.
#[component]
pub fn Tooltip(
    content: String,
    #[props(default)] open: Option<bool>,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut internal_open = use_signal(|| false);
    let is_open = match open {
        Some(v) => v,
        None => internal_open(),
    };

    let base_id = test_id.as_deref().unwrap_or("nve-tooltip");
    let trigger_id = format!("{base_id}-trigger");
    let bubble_id = format!("{base_id}-bubble");

    let mut wrap_sb = StyleBuf::new();
    wrap_sb.flex_direction("column").gap(theme.spacing.xs);
    if let Some(ref extra) = style {
        wrap_sb.raw(extra);
    }
    let wrap_style = wrap_sb.finish();

    let mut tip_sb = StyleBuf::new();
    tip_sb
        .padding_xy(theme.spacing.sm, theme.spacing.xs)
        .z_index(950)
        .surface(SurfaceStyle {
            background: theme.palette.text_primary.as_ref(),
            border_width: if theme.is_high_contrast() { 1.0 } else { 0.0 },
            border_color: if theme.is_high_contrast() {
                theme.palette.border.as_ref()
            } else {
                "transparent"
            },
            border_radius: theme.radii.sm,
            opacity: 1.0,
            box_shadow: theme.elevation_shadow(),
        });
    let tip_style = tip_sb.finish();

    let mut tip_ts = StyleBuf::new();
    tip_ts.text_style(TextStyle {
        color: theme.palette.surface.as_ref(),
        font_size: theme.typography.xs,
        font_weight: "500",
        text_align: "left",
        opacity: 1.0,
    });
    let tip_text_style = tip_ts.finish();

    let aria_expanded = if is_open { "true" } else { "false" };
    let described_by = if is_open {
        Some(bubble_id.clone())
    } else {
        None
    };

    rsx! {
        view {
            test_id: "{base_id}",
            style: "{wrap_style}",
            pressable {
                test_id: "{trigger_id}",
                tab_index: 0,
                aria_expanded: "{aria_expanded}",
                aria_describedby: described_by,
                onfocus: move |_| {
                    internal_open.set(true);
                    if let Some(handler) = on_open_change {
                        handler.call(true);
                    }
                },
                onblur: move |_| {
                    internal_open.set(false);
                    if let Some(handler) = on_open_change {
                        handler.call(false);
                    }
                },
                onkeydown: move |ev: KeyEvent| {
                    let data = ev.data();
                    let key = data.key.as_str();
                    if is_open && matches!(key, "Escape" | "Esc") {
                        internal_open.set(false);
                        if let Some(handler) = on_open_change {
                            handler.call(false);
                        }
                    }
                },
                {children}
            }
            if is_open {
                view {
                    test_id: "{bubble_id}",
                    accessibility_role: "tooltip",
                    style: "{tip_style}",
                    text {
                        style: "{tip_text_style}",
                        "{content}"
                    }
                }
            }
        }
    }
}
