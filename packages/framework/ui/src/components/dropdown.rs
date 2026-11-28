//! Accessible [`DropdownMenu`] with `menu` / `menuitem` ARIA roles, [`RovingFocus`]
//! vertical arrow/`Home`/`End` navigation skipping disabled items, `Enter` activation,
//! and `Escape` dismissal.

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::{ColorRgba, FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;

use crate::components::overlay_close::{use_close_latch, CloseLatch};
use crate::components::overlays::resolve_anchored_position;
use crate::focus::{FocusableEntry, Orientation, RovingFocus};
use crate::positioning::{Placement, PositionOptions, Rect, Size, Viewport};
use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};
use crate::components::roles::error_text;
use crate::theme::{use_theme, Layer};

/// Item entry rendered inside [`DropdownMenu`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub disabled: bool,
    pub destructive: bool,
}

impl MenuItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled: false,
            destructive: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }
}

fn dispatch_menu_key(
    ev: &KeyEvent,
    mut roving: Signal<RovingFocus>,
    latch: CloseLatch,
    on_select: Option<EventHandler<String>>,
    on_open_change: Option<EventHandler<bool>>,
) {
    match ev.data().key.as_str() {
        "Escape" | "Esc" => latch.close_on_escape(ev, on_open_change),
        "Enter" | " " | "Space" if latch.is_open() => {
            ev.stop_propagation();
            let selected = roving.read().active_id().map(str::to_owned);
            if let Some(id) = selected {
                if let Some(handler) = on_select {
                    handler.call(id);
                }
                let _ = latch.close(on_open_change);
            }
        }
        key @ ("ArrowUp" | "ArrowDown" | "Home" | "End") if latch.is_open() => {
            ev.stop_propagation();
            let _ = roving.write().handle_key(key);
        }
        _ => {}
    }
}

/// Accessible dropdown menu supporting keyboard roving focus (`ArrowUp`/`ArrowDown`/
/// `Home`/`End`), `Enter`/`Space` item selection, `Escape` dismissal, and optional
/// collision-flipping positioning via [`crate::positioning::compute_overlay_position`].
#[component]
pub fn DropdownMenu(
    open: bool,
    trigger_label: String,
    items: Vec<MenuItem>,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default)] on_select: Option<EventHandler<String>>,
    #[props(default = "auto".to_owned())] popover_mode: String,
    #[props(default)] keep_mounted: bool,
    #[props(default)] anchor_rect: Option<Rect>,
    #[props(default)] overlay_size: Option<Size>,
    #[props(default)] viewport: Option<Viewport>,
    #[props(default)] position_options: Option<PositionOptions>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let theme = use_theme();
    let mut trigger_focused = use_signal(|| false);

    let entries: Vec<FocusableEntry> = items
        .iter()
        .map(|it| FocusableEntry::disabled(it.id.clone(), it.disabled))
        .collect();

    let mut roving = use_signal(|| RovingFocus::with_items(Orientation::Vertical, entries.clone()));
    let latch = use_close_latch(open);

    // Keep roving item list synchronized without triggering render loops.
    {
        let mut rv = roving.write_unchecked();
        if rv.items() != entries.as_slice() {
            rv.set_items(entries);
        }
        if latch.opened
            && let Some(idx) = rv.first_enabled()
        {
            let _ = rv.set_active_index(idx);
        }
    }

    let base_id = test_id.as_deref().unwrap_or("nve-dropdown");
    let trigger_id = format!("{base_id}-trigger");
    let menu_id = format!("{base_id}-menu");

    let default_opts = position_options.unwrap_or_else(|| {
        PositionOptions::new(Placement::BottomStart).with_offset(4.0)
    });
    let est_height = (items.len().max(1) as f32 * 36.0) + theme.spacing.xs * 2.0;
    let computed = resolve_anchored_position(
        anchor_rect,
        overlay_size,
        viewport,
        Some(default_opts),
        Size::new(200.0, est_height),
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
            background: theme.surface.base,
            border_width: if trigger_focused() { 2.0 } else { 1.0 },
            border_color: if trigger_focused() {
                theme.border.focus
            } else {
                theme.border.default
            },
            border_radius: theme.radius.md,
            opacity: 1.0,
            box_shadow: None,
        })
        .finish();

    let trigger_text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: theme.text.primary,
            font_size: theme.typography.body.size,
            font_weight: "500",
            text_align: "center",
            opacity: 1.0,
        })
        .finish();

    let mut menu_builder = TypedStyleBuilder::new()
        .flex_col()
        .min_width_px(180.0)
        .padding(theme.spacing.xs)
        .gap(2.0)
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
            menu_builder = menu_builder.max_width_px(mw);
        }
        if let Some(mh) = pos.max_height {
            menu_builder = menu_builder.max_height_px(mh);
        }
    }
    let menu_style = menu_builder.finish();

    let pos_placement = computed.map(|p| p.placement.as_str().to_owned());
    let pos_x = computed.map(|p| format!("{:.2}", p.x));
    let pos_y = computed.map(|p| format!("{:.2}", p.y));
    let pos_flipped = computed.map(|p| if p.flipped { "true" } else { "false" }.to_owned());
    let pos_shifted = computed.map(|p| if p.shifted { "true" } else { "false" }.to_owned());
    let pos_arrow_x = computed.and_then(|p| p.arrow_x).map(|v| format!("{v:.2}"));
    let pos_arrow_y = computed.and_then(|p| p.arrow_y).map(|v| format!("{v:.2}"));

    let aria_expanded = if open { "true" } else { "false" };
    let active_idx = roving.read().active_index();

    let mut rendered_items = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let is_active = active_idx == Some(idx) && !item.disabled;
        let item_tab_index = roving.read().tab_index_for(idx);
        let item_test_id = format!("{base_id}-item-{}", item.id);

        let bg = if is_active {
            theme.surface.subtle
        } else {
            ColorRgba::TRANSPARENT
        };
        let bc = if is_active {
            theme.border.focus
        } else {
            ColorRgba::TRANSPARENT
        };
        let fg = if item.destructive {
            error_text(&theme)
        } else {
            theme.text.primary
        };

        let item_style = TypedStyleBuilder::new()
            .padding_xy(theme.spacing.md, theme.spacing.sm)
            .justify_content(FlexJustify::FlexStart)
            .align_items(FlexAlign::Center)
            .flex_row()
            .surface(TypedSurfaceStyle {
                background: bg,
                border_width: if is_active { 1.0 } else { 0.0 },
                border_color: bc,
                border_radius: theme.radius.sm,
                opacity: if item.disabled { 0.45 } else { 1.0 },
                box_shadow: None,
            })
            .finish();

        let item_text_style = TypedStyleBuilder::new()
            .text_style(TypedTextStyle {
                color: fg,
                font_size: theme.typography.ui.size,
                font_weight: if is_active { "600" } else { "400" },
                text_align: "left",
                opacity: 1.0,
            })
            .finish();

        let item_id = item.id.clone();
        let item_label = item.label.clone();
        let item_disabled = item.disabled;

        rendered_items.push(rsx! {
            pressable {
                key: "{item_id}",
                test_id: "{item_test_id}",
                accessibility_role: "menuitem",
                disabled: item_disabled,
                tab_index: item_tab_index,
                nve_style: item_style,
                onpress: move |_| {
                    if !item_disabled && latch.is_open() {
                        let _ = roving.write().set_active_index(idx);
                        if let Some(handler) = on_select {
                            handler.call(item_id.clone());
                        }
                        let _ = latch.close(on_open_change);
                    }
                },
                onkeydown: move |ev: KeyEvent| {
                    dispatch_menu_key(&ev, roving, latch, on_select, on_open_change);
                },
                text {
                    nve_style: item_text_style,
                    "{item_label}"
                }
            }
        });
    }

    let menu_trigger_invoker = super::html_tables::html_invoker_table(
        Some("button".to_owned()),
        None,
        None,
        Some("toggle-popover".to_owned()),
        Some(menu_id.clone()),
        Some(menu_id.clone()),
        Some("toggle".to_owned()),
    );
    let mut menu_data = Vec::new();
    if let Some(ref p) = pos_placement {
        menu_data.push(("placement".to_owned(), p.clone()));
    }
    if let Some(ref x) = pos_x {
        menu_data.push(("x".to_owned(), x.clone()));
    }
    if let Some(ref y) = pos_y {
        menu_data.push(("y".to_owned(), y.clone()));
    }
    if let Some(ref f) = pos_flipped {
        menu_data.push(("flipped".to_owned(), f.clone()));
    }
    if let Some(ref s) = pos_shifted {
        menu_data.push(("shifted".to_owned(), s.clone()));
    }
    if let Some(ref ax) = pos_arrow_x {
        menu_data.push(("arrow-x".to_owned(), ax.clone()));
    }
    if let Some(ref ay) = pos_arrow_y {
        menu_data.push(("arrow-y".to_owned(), ay.clone()));
    }
    let menu_popover_table =
        super::html_tables::html_popover_table(popover_mode.to_owned(), open, menu_data);

    rsx! {
        view {
            test_id: "{base_id}",
            nve_style: wrap_style,
            style: style,
            pressable {
                test_id: "{trigger_id}",
                nve_table: menu_trigger_invoker,
                accessibility_role: "button",
                aria_expanded: "{aria_expanded}",
                aria_controls: "{menu_id}",
                tab_index: 0,
                nve_style: trigger_style,
                onfocus: move |_| trigger_focused.set(true),
                onblur: move |_| trigger_focused.set(false),
                onpress: move |_| latch.toggle(on_open_change),
                onkeydown: move |ev: KeyEvent| {
                    if !latch.is_open() {
                        if matches!(ev.data().key.as_str(), "ArrowDown" | "Enter" | " " | "Space") {
                            ev.stop_propagation();
                            if let Some(handler) = on_open_change {
                                handler.call(true);
                            }
                        }
                    } else {
                        dispatch_menu_key(&ev, roving, latch, on_select, on_open_change);
                    }
                },
                text {
                    nve_style: trigger_text_style,
                    "{trigger_label}"
                }
            }
            if open || keep_mounted {
                view {
                    id: "{menu_id}",
                    test_id: "{menu_id}",
                    nve_table: menu_popover_table,
                    accessibility_role: "menu",
                    tab_index: -1,
                    nve_style: menu_style,
                    // Native light dismiss / `Esc` (target-only `close`) syncs `open` and
                    // `aria-expanded`, exactly like `Popover`.
                    onclose: move |_: CustomEvent| {
                        let _ = latch.close(on_open_change);
                    },
                    onkeydown: move |ev: KeyEvent| {
                        dispatch_menu_key(&ev, roving, latch, on_select, on_open_change);
                    },
                    {rendered_items.into_iter()}
                }
            }
        }
    }
}
