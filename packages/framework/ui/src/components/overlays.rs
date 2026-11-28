//! Accessible overlay components: [`Dialog`] / [`Modal`], [`Popover`], [`Tooltip`],
//! and re-exported [`DropdownMenu`] / [`MenuItem`].

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::{ColorRgba, FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;

pub use crate::components::dropdown::{DropdownMenu, MenuItem};
pub use crate::components::tooltip::Tooltip;
use crate::focus::{
    current_focused_id, notify_focus_scope, FocusTrap, FocusTrapAction, FocusableEntry,
};
use crate::components::overlay_close::use_close_latch;
use crate::positioning::{
    compute_overlay_position, ComputedPosition, PositionOptions, Rect, Size, Viewport,
};
use crate::surface::{SurfaceStyleExt, TypedSurfaceStyle, TypedTextStyle};
use crate::theme::{use_theme, Layer};

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

/// Accessible modal dialog emitting native HTML5 `<dialog closedby="...">` + Invoker Commands
/// (`<button command="show-modal" commandfor="...">` / `<button command="close" commandfor="...">`)
/// on Web while preserving native Skia/Headless [`FocusTrap`] (`Tab`/`Shift+Tab` wrap-around +
/// focus restoration), `Escape` dismissal, and backdrop scrim dismissal.
#[component]
pub fn Dialog(
    open: bool,
    #[props(default)] id: Option<String>,
    #[props(default)] title: Option<String>,
    #[props(default)] description: Option<String>,
    #[props(default)] trigger_label: Option<String>,
    #[props(default)] close_label: Option<String>,
    #[props(default)] keep_mounted: bool,
    #[props(default)] focusable_ids: Vec<String>,
    #[props(default)] previous_focus: Option<String>,
    #[props(default = true)] close_on_escape: bool,
    #[props(default = true)] close_on_backdrop: bool,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default)] on_close: Option<EventHandler<()>>,
    #[props(default)] on_focus_change: Option<EventHandler<Option<String>>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] nve_style: Option<TypedStyle>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut trap = use_signal(|| {
        FocusTrap::from_ids(focusable_ids.iter().cloned()).with_close_on_escape(close_on_escape)
    });
    let latch = use_close_latch(open);

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

    if latch.opened {
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
    } else if latch.closed {
        if trap.peek().is_open() {
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
    } else if open && !trap.peek().is_open() {
        // Parent vetoed a dismiss (or omitted callbacks): re-arm the trap silently for the next
        // `Tab`/`Escape` without firing a spurious `on_focus_change`.
        let prev = previous_focus.clone().or_else(current_focused_id);
        let mut t = trap.write_unchecked();
        let _ = t.activate(prev);
    }

    let has_invoker = trigger_label.is_some();
    if !open && !has_invoker && !keep_mounted {
        return rsx! {};
    }

    let base_id = test_id.as_deref().unwrap_or("nve-dialog");
    let dialog_id = id.as_deref().unwrap_or(base_id).to_owned();
    let trigger_id = format!("{base_id}-trigger");
    let close_btn_id = format!("{base_id}-close");
    let backdrop_id = format!("{base_id}-backdrop");
    let title_id = title.as_ref().map(|_| format!("{base_id}-title"));
    let desc_id = description.as_ref().map(|_| format!("{base_id}-description"));
    let closedby = match (close_on_backdrop, close_on_escape) {
        (true, true) => "any",
        (false, true) => "closerequest",
        _ => "none",
    };

    let trigger_style = TypedStyle::new()
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .surface(TypedSurfaceStyle {
            background: theme.surface.base,
            border_width: 1.0,
            border_color: theme.border.default,
            border_radius: theme.radius.md,
            opacity: 1.0,
            box_shadow: None,
        });
    let trigger_text_style = TypedStyle::new().text_style(TypedTextStyle {
        color: theme.text.primary,
        font_size: theme.typography.body.size,
        font_weight: 500,
        text_align: TextAlign::Center,
        opacity: 1.0,
    });

    let backdrop_style = TypedStyle::new()
        .width_pct(100.0)
        .height_pct(100.0)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .padding(theme.spacing.xl)
        .z_index(Layer::Modal.z())
        .surface(TypedSurfaceStyle {
            background: theme.scrim,
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: 0.0,
            opacity: 1.0,
            box_shadow: None,
        });

    let panel_style = TypedStyle::new()
        .flex_col()
        .min_width_px(320.0)
        .max_width_px(560.0)
        .padding(theme.spacing.xl)
        .gap(theme.spacing.md)
        .z_index(Layer::Modal.z() + 1)
        .surface(TypedSurfaceStyle {
            background: theme.surface.overlay,
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: theme.border.default,
            border_radius: theme.radius.lg,
            opacity: 1.0,
            box_shadow: theme.elevation.overlay,
        })
        .merge_opt(nve_style.as_ref());

    let title_style = TypedStyle::new().text_style(TypedTextStyle {
        color: theme.text.primary,
        font_size: theme.typography.title.size,
        font_weight: 700,
        text_align: TextAlign::Left,
        opacity: 1.0,
    });

    let desc_style = TypedStyle::new().text_style(TypedTextStyle {
        color: theme.text.secondary,
        font_size: theme.typography.ui.size,
        font_weight: 400,
        text_align: TextAlign::Left,
        opacity: 1.0,
    });

    // Every dismissal path (close button, scrim, `Escape`, native `close`) funnels through here,
    // gated by `CloseLatch` so same-batch app + native closes fire at most once per open period.
    let mut dismiss = move |pre_restored: Option<Option<String>>| -> bool {
        if !latch.is_open() {
            return false;
        }
        let restored = pre_restored.unwrap_or_else(|| trap.write().deactivate());
        if let Some(ref fid) = restored {
            notify_focus_scope(fid);
        }
        if let Some(handler) = on_focus_change {
            handler.call(restored);
        }
        let _ = latch.close(on_open_change);
        if let Some(handler) = on_close {
            handler.call(());
        }
        true
    };

    let active_focus = trap.read().active().map(str::to_owned);
    let dialog_id_for_close = dialog_id.clone();
    let dialog_table = super::html_tables::html_dialog_table(open, closedby.to_owned());
    let close_invoker = super::html_tables::html_invoker_table(
        Some("button".to_owned()),
        None,
        None,
        Some("close".to_owned()),
        Some(dialog_id_for_close),
        None,
        None,
    );
    let trigger_invoker = super::html_tables::html_invoker_table(
        Some("button".to_owned()),
        None,
        None,
        Some("show-modal".to_owned()),
        Some(dialog_id.clone()),
        None,
        None,
    );

    let dialog_el = rsx! {
        view {
            id: "{dialog_id}",
            test_id: "{base_id}",
            nve_table: dialog_table,
            accessibility_role: "dialog",
            accessibility_label: accessibility_label,
            aria_modal: "true",
            aria_labelledby: title_id.clone(),
            aria_describedby: desc_id.clone(),
            aria_controls: active_focus,
            tab_index: -1,
            nve_style: panel_style,
            onpress: move |ev: PressEvent| ev.stop_propagation(),
            // Browser-initiated close (native `closedby` backdrop/`Esc`, form `method=dialog`),
            // delivered target-only like the DOM `close`; gated by `CloseLatch` against same-batch
            // app-driven closes.
            onclose: move |_: CustomEvent| {
                let _ = dismiss(None);
            },
            onkeydown: move |ev: KeyEvent| {
                if !latch.is_open() {
                    return;
                }
                let data = ev.data();
                let action = trap.write().handle_key(&data.key, data.shift);
                match action {
                    FocusTrapAction::FocusChanged(next_id) => {
                        ev.stop_propagation();
                        notify_focus_scope(&next_id);
                        if let Some(handler) = on_focus_change {
                            handler.call(Some(next_id));
                        }
                    }
                    FocusTrapAction::Dismiss { restore_focus } => {
                        if dismiss(Some(restore_focus)) {
                            ev.stop_propagation();
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
            if let Some(ref close_txt) = close_label {
                button {
                    test_id: "{close_btn_id}",
                    nve_table: close_invoker,
                    accessibility_role: "button",
                    tab_index: 0,
                    nve_style: trigger_style.clone(),
                    onpress: move |_| {
                        let _ = dismiss(None);
                    },
                    text {
                        nve_style: trigger_text_style.clone(),
                        "{close_txt}"
                    }
                }
            }
        }
    };

    if has_invoker || keep_mounted {
        return rsx! {
            view {
                if let Some(ref trig) = trigger_label {
                    button {
                        test_id: "{trigger_id}",
                        nve_table: trigger_invoker,
                        accessibility_role: "button",
                        tab_index: 0,
                        nve_style: trigger_style,
                        onpress: move |_| {
                            if let Some(handler) = on_open_change {
                                handler.call(true);
                            }
                        },
                        text {
                            nve_style: trigger_text_style,
                            "{trig}"
                        }
                    }
                }
                {dialog_el}
            }
        };
    }

    // Controlled modal scrim wrapper preserving native Skia/Headless backdrop dismissal.
    rsx! {
        view {
            test_id: "{backdrop_id}",
            accessibility_role: "presentation",
            nve_style: backdrop_style,
            onpress: move |_| {
                if close_on_backdrop {
                    let _ = dismiss(None);
                }
            },
            {dialog_el}
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
    #[props(default = "auto".to_owned())] popover_mode: String,
    #[props(default)] keep_mounted: bool,
    #[props(default)] anchor_rect: Option<Rect>,
    #[props(default)] overlay_size: Option<Size>,
    #[props(default)] viewport: Option<Viewport>,
    #[props(default)] position_options: Option<PositionOptions>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] nve_style: Option<TypedStyle>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut focused = use_signal(|| false);
    let latch = use_close_latch(open);

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

    let wrap_style = TypedStyle::new()
        .flex_col()
        .gap(theme.spacing.xs)
        .merge_opt(nve_style.as_ref());

    let trigger_style = TypedStyle::new()
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .surface(TypedSurfaceStyle {
            background: theme.surface.base,
            border_width: if focused() { 2.0 } else { 1.0 },
            border_color: if focused() {
                theme.border.focus
            } else {
                theme.border.default
            },
            border_radius: theme.radius.md,
            opacity: 1.0,
            box_shadow: None,
        });

    let trigger_text_style = TypedStyle::new().text_style(TypedTextStyle {
        color: theme.text.primary,
        font_size: theme.typography.body.size,
        font_weight: 500,
        text_align: TextAlign::Center,
        opacity: 1.0,
    });

    let mut panel_style = TypedStyle::new()
        .flex_col()
        .padding(theme.spacing.md)
        .gap(theme.spacing.sm)
        .z_index(Layer::Popover.z())
        .surface(TypedSurfaceStyle {
            background: theme.surface.overlay,
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: theme.border.default,
            border_radius: theme.radius.md,
            opacity: 1.0,
            box_shadow: theme.elevation.overlay,
        });
    if let Some(pos) = computed {
        if let Some(mw) = pos.max_width {
            panel_style = panel_style.max_width_px(mw);
        }
        if let Some(mh) = pos.max_height {
            panel_style = panel_style.max_height_px(mh);
        }
    }

    let pos_placement = computed.map(|p| p.placement.as_str().to_owned());
    let pos_x = computed.map(|p| format!("{:.2}", p.x));
    let pos_y = computed.map(|p| format!("{:.2}", p.y));
    let pos_flipped = computed.map(|p| if p.flipped { "true" } else { "false" }.to_owned());
    let pos_shifted = computed.map(|p| if p.shifted { "true" } else { "false" }.to_owned());
    let pos_arrow_x = computed.and_then(|p| p.arrow_x).map(|v| format!("{v:.2}"));
    let pos_arrow_y = computed.and_then(|p| p.arrow_y).map(|v| format!("{v:.2}"));

    let aria_expanded = if open { "true" } else { "false" };

    let popover_trigger_invoker = super::html_tables::html_invoker_table(
        Some("button".to_owned()),
        None,
        None,
        Some("toggle-popover".to_owned()),
        Some(content_id.clone()),
        Some(content_id.clone()),
        Some("toggle".to_owned()),
    );
    let mut popover_data = Vec::new();
    if let Some(ref p) = pos_placement {
        popover_data.push(("placement".to_owned(), p.clone()));
    }
    if let Some(ref x) = pos_x {
        popover_data.push(("x".to_owned(), x.clone()));
    }
    if let Some(ref y) = pos_y {
        popover_data.push(("y".to_owned(), y.clone()));
    }
    if let Some(ref f) = pos_flipped {
        popover_data.push(("flipped".to_owned(), f.clone()));
    }
    if let Some(ref s) = pos_shifted {
        popover_data.push(("shifted".to_owned(), s.clone()));
    }
    if let Some(ref ax) = pos_arrow_x {
        popover_data.push(("arrow-x".to_owned(), ax.clone()));
    }
    if let Some(ref ay) = pos_arrow_y {
        popover_data.push(("arrow-y".to_owned(), ay.clone()));
    }
    let popover_table =
        super::html_tables::html_popover_table(popover_mode.to_owned(), open, popover_data);

    rsx! {
        view {
            test_id: "{base_id}",
            nve_style: wrap_style,
            pressable {
                test_id: "{trigger_id}",
                nve_table: popover_trigger_invoker,
                accessibility_role: "button",
                aria_expanded: "{aria_expanded}",
                aria_controls: "{content_id}",
                tab_index: 0,
                nve_style: trigger_style,
                onfocus: move |_| focused.set(true),
                onblur: move |_| focused.set(false),
                onpress: move |_| latch.toggle(on_open_change),
                onkeydown: move |ev: KeyEvent| {
                    match ev.data().key.as_str() {
                        "Escape" | "Esc" if close_on_escape => {
                            latch.close_on_escape(&ev, on_open_change);
                        }
                        "Enter" | " " | "Space" => latch.toggle(on_open_change),
                        _ => {}
                    }
                },
                text {
                    nve_style: trigger_text_style,
                    "{trigger_label}"
                }
            }
            if open || keep_mounted {
                view {
                    id: "{content_id}",
                    test_id: "{content_id}",
                    nve_table: popover_table,
                    accessibility_role: "dialog",
                    tab_index: -1,
                    nve_style: panel_style,
                    // Native light dismiss / `Esc` / `popovertargetaction=hide` (target-only).
                    onclose: move |_: CustomEvent| {
                        let _ = latch.close(on_open_change);
                    },
                    onkeydown: move |ev: KeyEvent| {
                        if close_on_escape {
                            latch.close_on_escape(&ev, on_open_change);
                        }
                    },
                    {children}
                }
            }
        }
    }
}
