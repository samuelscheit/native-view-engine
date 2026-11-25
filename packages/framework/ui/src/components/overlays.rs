//! Accessible overlay components: [`Dialog`] / [`Modal`], [`Popover`], [`Tooltip`],
//! and re-exported [`DropdownMenu`] / [`MenuItem`].

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::{ColorRgba, FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;

pub use crate::components::dropdown::{DropdownMenu, MenuItem};
use crate::focus::{
    current_focused_id, notify_focus_scope, FocusTrap, FocusTrapAction, FocusableEntry,
};
use crate::positioning::{
    compute_overlay_position, ComputedPosition, PositionOptions, Rect, Size, Viewport,
};
use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};
use crate::theme::use_theme_rgba;

#[inline]
pub(crate) fn resolve_anchored_position(
    anchor_rect: Option<Rect>,
    overlay_size: Option<Size>,
    viewport: Option<Viewport>,
    position_options: Option<PositionOptions>,
    default_size: Size,
) -> Option<ComputedPosition> {
    let (anchor, vp) = (anchor_rect?, viewport?);
    let size = overlay_size.unwrap_or(default_size);
    let opts = position_options.unwrap_or_default();
    Some(compute_overlay_position(anchor, size, vp, &opts))
}

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
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let (theme, colors) = use_theme_rgba();
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

    let backdrop_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .height_pct(100.0)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .padding(theme.spacing.xl)
        .z_index(1000)
        .surface(TypedSurfaceStyle {
            background: theme.backdrop_scrim_rgba(),
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: 0.0,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let panel_style = TypedStyleBuilder::new()
        .flex_col()
        .min_width_px(320.0)
        .max_width_px(560.0)
        .padding(theme.spacing.xl)
        .gap(theme.spacing.md)
        .z_index(1001)
        .surface(TypedSurfaceStyle {
            background: colors.surface_elevated,
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: colors.border,
            border_radius: theme.radii.lg,
            opacity: 1.0,
            box_shadow: theme.elevation_shadow(),
        })
        .finish();

    let title_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.lg,
            font_weight: "700",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let desc_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_secondary,
            font_size: theme.typography.sm,
            font_weight: "400",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let active_focus = trap.read().active().map(str::to_owned);

    // The scrim is a plain `view` with a press listener (a `pressable` would nest the panel's
    // buttons and text fields inside a `<button>`); the panel swallows presses so a click on its
    // padding or a text field never bubbles to the scrim and dismisses the dialog.
    rsx! {
        view {
            test_id: "{backdrop_id}",
            accessibility_role: "presentation",
            nve_style: backdrop_style,
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
                accessibility_label: accessibility_label,
                aria_modal: "true",
                aria_labelledby: title_id.clone(),
                aria_describedby: desc_id.clone(),
                aria_controls: active_focus,
                tab_index: -1,
                nve_style: panel_style,
                style: style,
                onpress: move |ev: PressEvent| ev.stop_propagation(),
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
                        nve_style: title_style,
                        "{heading}"
                    }
                }
                if let Some(ref desc) = description {
                    text {
                        test_id: desc_id.clone(),
                        nve_style: desc_style,
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

/// Accessible anchored popover (`aria-expanded`, `aria-controls`, `Escape` dismiss) with
/// optional collision-flipping positioning via [`compute_overlay_position`].
#[component]
pub fn Popover(
    open: bool,
    trigger_label: String,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default = true)] close_on_escape: bool,
    #[props(default)] anchor_rect: Option<Rect>,
    #[props(default)] overlay_size: Option<Size>,
    #[props(default)] viewport: Option<Viewport>,
    #[props(default)] position_options: Option<PositionOptions>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let (theme, colors) = use_theme_rgba();
    let mut focused = use_signal(|| false);

    let base_id = test_id.as_deref().unwrap_or("nve-popover");
    let trigger_id = format!("{base_id}-trigger");
    let content_id = format!("{base_id}-content");

    let computed = resolve_anchored_position(
        anchor_rect,
        overlay_size,
        viewport,
        position_options,
        Size::new(240.0, 140.0),
    );

    let wrap_style = TypedStyleBuilder::new()
        .flex_col()
        .gap(theme.spacing.xs)
        .finish();

    let trigger_style = TypedStyleBuilder::new()
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .surface(TypedSurfaceStyle {
            background: colors.surface,
            border_width: if focused() { 2.0 } else { 1.0 },
            border_color: if focused() {
                colors.border_focus
            } else {
                colors.border
            },
            border_radius: theme.radii.md,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let trigger_text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.md,
            font_weight: "500",
            text_align: "center",
            opacity: 1.0,
        })
        .finish();

    let mut panel_builder = TypedStyleBuilder::new()
        .flex_col()
        .padding(theme.spacing.md)
        .gap(theme.spacing.sm)
        .z_index(900)
        .surface(TypedSurfaceStyle {
            background: colors.surface_elevated,
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: colors.border,
            border_radius: theme.radii.md,
            opacity: 1.0,
            box_shadow: theme.elevation_shadow(),
        });
    if let Some(pos) = computed {
        if let Some(mw) = pos.max_width {
            panel_builder = panel_builder.max_width_px(mw);
        }
        if let Some(mh) = pos.max_height {
            panel_builder = panel_builder.max_height_px(mh);
        }
    }
    let panel_style = panel_builder.finish();

    let pos_placement = computed.map(|p| p.placement.as_str().to_owned());
    let pos_x = computed.map(|p| format!("{:.2}", p.x));
    let pos_y = computed.map(|p| format!("{:.2}", p.y));
    let pos_flipped = computed.map(|p| if p.flipped { "true" } else { "false" }.to_owned());
    let pos_shifted = computed.map(|p| if p.shifted { "true" } else { "false" }.to_owned());
    let pos_arrow_x = computed.and_then(|p| p.arrow_x).map(|v| format!("{v:.2}"));
    let pos_arrow_y = computed.and_then(|p| p.arrow_y).map(|v| format!("{v:.2}"));

    let aria_expanded = if open { "true" } else { "false" };

    rsx! {
        view {
            test_id: "{base_id}",
            nve_style: wrap_style,
            style: style,
            pressable {
                test_id: "{trigger_id}",
                accessibility_role: "button",
                aria_expanded: "{aria_expanded}",
                aria_controls: "{content_id}",
                tab_index: 0,
                nve_style: trigger_style,
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
                    nve_style: trigger_text_style,
                    "{trigger_label}"
                }
            }
            if open {
                view {
                    test_id: "{content_id}",
                    accessibility_role: "dialog",
                    tab_index: -1,
                    "data-placement": pos_placement,
                    "data-x": pos_x,
                    "data-y": pos_y,
                    "data-flipped": pos_flipped,
                    "data-shifted": pos_shifted,
                    "data-arrow-x": pos_arrow_x,
                    "data-arrow-y": pos_arrow_y,
                    nve_style: panel_style,
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
/// `Escape` dismiss) with optional anchored collision-flipping via [`compute_overlay_position`].
/// Works in both controlled (`open: Some(bool)`) and uncontrolled modes.
#[component]
pub fn Tooltip(
    content: String,
    #[props(default)] open: Option<bool>,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default)] anchor_rect: Option<Rect>,
    #[props(default)] overlay_size: Option<Size>,
    #[props(default)] viewport: Option<Viewport>,
    #[props(default)] position_options: Option<PositionOptions>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let (theme, colors) = use_theme_rgba();
    let mut internal_open = use_signal(|| false);
    let is_open = match open {
        Some(v) => v,
        None => internal_open(),
    };

    let base_id = test_id.as_deref().unwrap_or("nve-tooltip");
    let trigger_id = format!("{base_id}-trigger");
    let bubble_id = format!("{base_id}-bubble");

    let default_tip_opts = position_options.unwrap_or_else(|| {
        PositionOptions::new(crate::positioning::Placement::Top).with_offset(6.0)
    });
    let computed = resolve_anchored_position(
        anchor_rect,
        overlay_size,
        viewport,
        Some(default_tip_opts),
        Size::new(160.0, 32.0),
    );

    let wrap_style = TypedStyleBuilder::new()
        .flex_col()
        .gap(theme.spacing.xs)
        .finish();

    let mut tip_builder = TypedStyleBuilder::new()
        .padding_xy(theme.spacing.sm, theme.spacing.xs)
        .z_index(950)
        .surface(TypedSurfaceStyle {
            background: colors.text_primary,
            border_width: if theme.is_high_contrast() { 1.0 } else { 0.0 },
            border_color: if theme.is_high_contrast() {
                colors.border
            } else {
                ColorRgba::TRANSPARENT
            },
            border_radius: theme.radii.sm,
            opacity: 1.0,
            box_shadow: theme.elevation_shadow(),
        });
    if let Some(pos) = computed {
        if let Some(mw) = pos.max_width {
            tip_builder = tip_builder.max_width_px(mw);
        }
        if let Some(mh) = pos.max_height {
            tip_builder = tip_builder.max_height_px(mh);
        }
    }
    let tip_style = tip_builder.finish();

    let pos_placement = computed.map(|p| p.placement.as_str().to_owned());
    let pos_x = computed.map(|p| format!("{:.2}", p.x));
    let pos_y = computed.map(|p| format!("{:.2}", p.y));
    let pos_flipped = computed.map(|p| if p.flipped { "true" } else { "false" }.to_owned());
    let pos_shifted = computed.map(|p| if p.shifted { "true" } else { "false" }.to_owned());
    let pos_arrow_x = computed.and_then(|p| p.arrow_x).map(|v| format!("{v:.2}"));
    let pos_arrow_y = computed.and_then(|p| p.arrow_y).map(|v| format!("{v:.2}"));

    let tip_text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.surface,
            font_size: theme.typography.xs,
            font_weight: "500",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let aria_expanded = if is_open { "true" } else { "false" };
    let described_by = if is_open {
        Some(bubble_id.clone())
    } else {
        None
    };

    rsx! {
        view {
            test_id: "{base_id}",
            nve_style: wrap_style,
            style: style,
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
                    "data-placement": pos_placement,
                    "data-x": pos_x,
                    "data-y": pos_y,
                    "data-flipped": pos_flipped,
                    "data-shifted": pos_shifted,
                    "data-arrow-x": pos_arrow_x,
                    "data-arrow-y": pos_arrow_y,
                    nve_style: tip_style,
                    text {
                        nve_style: tip_text_style,
                        "{content}"
                    }
                }
            }
        }
    }
}
