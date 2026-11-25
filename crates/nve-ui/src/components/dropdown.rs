//! Accessible [`DropdownMenu`] with `menu` / `menuitem` ARIA roles, [`RovingFocus`]
//! vertical arrow/`Home`/`End` navigation skipping disabled items, `Enter` activation,
//! and `Escape` dismissal.

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;

use crate::focus::{FocusableEntry, Orientation, RovingFocus};
use crate::style_buf::{StyleBuf, SurfaceStyle, TextStyle};
use crate::theme::use_theme;

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
    key: &str,
    mut roving: Signal<RovingFocus>,
    on_select: Option<EventHandler<String>>,
    on_open_change: Option<EventHandler<bool>>,
) {
    match key {
        "Escape" | "Esc" => {
            if let Some(handler) = on_open_change {
                handler.call(false);
            }
        }
        "Enter" | " " | "Space" => {
            let selected = roving.read().active_id().map(str::to_owned);
            if let Some(id) = selected {
                if let Some(handler) = on_select {
                    handler.call(id);
                }
                if let Some(handler) = on_open_change {
                    handler.call(false);
                }
            }
        }
        _ => {
            let _ = roving.write().handle_key(key);
        }
    }
}

/// Accessible dropdown menu supporting keyboard roving focus (`ArrowUp`/`ArrowDown`/
/// `Home`/`End`), `Enter`/`Space` item selection, and `Escape` dismissal.
#[component]
pub fn DropdownMenu(
    open: bool,
    trigger_label: String,
    items: Vec<MenuItem>,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default)] on_select: Option<EventHandler<String>>,
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
    let mut was_open = use_signal(|| false);

    // Keep roving item list synchronized without triggering render loops.
    {
        let mut rv = roving.write_unchecked();
        if rv.items() != entries.as_slice() {
            rv.set_items(entries);
        }
        if open && !*was_open.peek() {
            let first = rv.first_enabled();
            if let Some(idx) = first {
                let _ = rv.set_active_index(idx);
            }
        }
    }
    if open != *was_open.peek() {
        was_open.set(open);
    }

    let base_id = test_id.as_deref().unwrap_or("nve-dropdown");
    let trigger_id = format!("{base_id}-trigger");
    let menu_id = format!("{base_id}-menu");

    let mut wrap_sb = StyleBuf::new();
    wrap_sb.flex_direction("column").gap(theme.spacing.xs);
    if let Some(ref extra) = style {
        wrap_sb.raw(extra);
    }
    let wrap_style = wrap_sb.finish();

    let mut trig_sb = StyleBuf::new();
    trig_sb
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .justify_content("center")
        .align_items("center")
        .surface(SurfaceStyle {
            background: theme.palette.surface.as_ref(),
            border_width: if trigger_focused() { 2.0 } else { 1.0 },
            border_color: if trigger_focused() {
                theme.palette.border_focus.as_ref()
            } else {
                theme.palette.border.as_ref()
            },
            border_radius: theme.radii.md,
            opacity: 1.0,
            box_shadow: "none",
        });
    let trigger_style = trig_sb.finish();

    let mut trig_ts = StyleBuf::new();
    trig_ts.text_style(TextStyle {
        color: theme.palette.text_primary.as_ref(),
        font_size: theme.typography.md,
        font_weight: "500",
        text_align: "center",
        opacity: 1.0,
    });
    let trigger_text_style = trig_ts.finish();

    let mut menu_sb = StyleBuf::new();
    menu_sb
        .flex_direction("column")
        .min_width_px(180.0)
        .padding(theme.spacing.xs)
        .gap(2.0)
        .z_index(920)
        .surface(SurfaceStyle {
            background: theme.palette.surface_elevated.as_ref(),
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: theme.palette.border.as_ref(),
            border_radius: theme.radii.md,
            opacity: 1.0,
            box_shadow: theme.elevation_shadow(),
        });
    let menu_style = menu_sb.finish();

    let aria_expanded = if open { "true" } else { "false" };
    let active_idx = roving.read().active_index();

    let mut rendered_items = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let is_active = active_idx == Some(idx) && !item.disabled;
        let item_tab_index = roving.read().tab_index_for(idx);
        let item_test_id = format!("{base_id}-item-{}", item.id);

        let bg = if is_active {
            theme.palette.surface_subtle.as_ref()
        } else {
            "transparent"
        };
        let bc = if is_active {
            theme.palette.border_focus.as_ref()
        } else {
            "transparent"
        };
        let fg = if item.destructive {
            theme.palette.danger.as_ref()
        } else {
            theme.palette.text_primary.as_ref()
        };

        let mut item_sb = StyleBuf::new();
        item_sb
            .padding_xy(theme.spacing.md, theme.spacing.sm)
            .justify_content("flex-start")
            .align_items("center")
            .flex_direction("row")
            .surface(SurfaceStyle {
                background: bg,
                border_width: if is_active { 1.0 } else { 0.0 },
                border_color: bc,
                border_radius: theme.radii.sm,
                opacity: if item.disabled { 0.45 } else { 1.0 },
                box_shadow: "none",
            });
        let item_style = item_sb.finish();

        let mut item_ts = StyleBuf::new();
        item_ts.text_style(TextStyle {
            color: fg,
            font_size: theme.typography.sm,
            font_weight: if is_active { "600" } else { "400" },
            text_align: "left",
            opacity: 1.0,
        });
        let item_text_style = item_ts.finish();

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
                style: "{item_style}",
                onpress: move |_| {
                    if !item_disabled {
                        let _ = roving.write().set_active_index(idx);
                        if let Some(handler) = on_select {
                            handler.call(item_id.clone());
                        }
                        if let Some(handler) = on_open_change {
                            handler.call(false);
                        }
                    }
                },
                onkeydown: move |ev: KeyEvent| {
                    let data = ev.data();
                    dispatch_menu_key(data.key.as_str(), roving, on_select, on_open_change);
                },
                text {
                    style: "{item_text_style}",
                    "{item_label}"
                }
            }
        });
    }

    rsx! {
        view {
            test_id: "{base_id}",
            style: "{wrap_style}",
            pressable {
                test_id: "{trigger_id}",
                accessibility_role: "button",
                aria_expanded: "{aria_expanded}",
                aria_controls: "{menu_id}",
                tab_index: 0,
                style: "{trigger_style}",
                onfocus: move |_| trigger_focused.set(true),
                onblur: move |_| trigger_focused.set(false),
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
                            "ArrowDown" | "Enter" | " " | "Space" if !open => handler.call(true),
                            "Escape" | "Esc" if open => handler.call(false),
                            _ if open => dispatch_menu_key(key, roving, on_select, on_open_change),
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
                    test_id: "{menu_id}",
                    accessibility_role: "menu",
                    tab_index: -1,
                    style: "{menu_style}",
                    onkeydown: move |ev: KeyEvent| {
                        let data = ev.data();
                        dispatch_menu_key(data.key.as_str(), roving, on_select, on_open_change);
                    },
                    {rendered_items.into_iter()}
                }
            }
        }
    }
}
