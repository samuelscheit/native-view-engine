//! Accessible Tabs compound component: [`TabsRoot`], [`TabList`], [`TabTrigger`], and [`TabPanel`].
//!
//! Emits WAI-ARIA `tablist`/`tab`/`tabpanel`, `aria-selected`, `aria-controls`,
//! `aria-labelledby`, and roving `tabIndex` with keyboard arrow/`Home`/`End` navigation
//! powered by [`RovingFocus`].

use std::cell::RefCell;
use std::rc::Rc;

use dioxus_core::{provide_context, try_consume_context, use_hook, EventHandler};
use dioxus_core_macro::Props;
use nve_abi::{FlexAlign, FlexDirection, FlexJustify};
use nve_dioxus::prelude::*;

use crate::focus::{FocusableEntry, Orientation, RovingFocus};
use crate::surface::{SurfaceStyleExt, TypedSurfaceStyle, TypedTextStyle};
use crate::components::roles::{tab_colors, ControlColors};
use crate::theme::use_theme;

#[derive(Clone)]
struct TabsContext {
    active: Signal<String>,
    roster: Rc<RefCell<Vec<FocusableEntry>>>,
    orientation: Orientation,
    id_prefix: String,
    on_change: Option<EventHandler<String>>,
}

impl TabsContext {
    fn register_tab(&self, value: &str, disabled: bool) {
        let mut list = self.roster.borrow_mut();
        if let Some(entry) = list.iter_mut().find(|e| e.id == value) {
            entry.disabled = disabled;
        } else {
            list.push(FocusableEntry::disabled(value.to_owned(), disabled));
        }
    }

    fn select(&mut self, next: String) {
        if *self.active.peek() != next {
            self.active.set(next.clone());
            if let Some(handler) = self.on_change {
                handler.call(next);
            }
        }
    }

    fn navigate_key(&mut self, key: &str) {
        let items = self.roster.borrow().clone();
        let mut roving = RovingFocus::with_items(self.orientation, items);
        let cur = self.active.peek().clone();
        let _ = roving.set_active_id(&cur);
        if let Some(next_idx) = roving.handle_key(key)
            && let Some(entry) = roving.items().get(next_idx)
        {
            self.select(entry.id.clone());
        }
    }

    fn tab_id(&self, value: &str) -> String {
        format!("{}-tab-{value}", self.id_prefix)
    }

    fn panel_id(&self, value: &str) -> String {
        format!("{}-panel-{value}", self.id_prefix)
    }
}

/// Root provider for a tabbed interface.
#[component]
pub fn TabsRoot(
    #[props(default)] value: Option<String>,
    #[props(default)] default_value: String,
    #[props(default)] orientation: Orientation,
    #[props(default)] id_prefix: Option<String>,
    #[props(default)] on_change: Option<EventHandler<String>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] nve_style: Option<TypedStyle>,
    children: Element,
) -> Element {
    let initial = value.clone().unwrap_or_else(|| default_value.clone());
    let prefix = id_prefix.unwrap_or_else(|| "tabs".to_owned());

    let mut ctx = use_hook(|| {
        let active = Signal::new(initial.clone());
        provide_context(TabsContext {
            active,
            roster: Rc::new(RefCell::new(Vec::new())),
            orientation,
            id_prefix: prefix.clone(),
            on_change,
        })
    });

    if let Some(ref controlled) = value
        && *ctx.active.peek() != *controlled
    {
        ctx.active.set(controlled.clone());
    }

    let root_style = TypedStyle::new()
        .flex_direction(match orientation {
            Orientation::Horizontal => FlexDirection::Column,
            Orientation::Vertical => FlexDirection::Row,
        })
        .gap(8.0)
        .width_pct(100.0)
        .merge_opt(nve_style.as_ref());

    rsx! {
        view {
            test_id: test_id,
            nve_style: root_style,
            {children}
        }
    }
}

/// Container for [`TabTrigger`] items (`accessibilityRole="tablist"`).
#[component]
pub fn TabList(
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] nve_style: Option<TypedStyle>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let orientation = try_consume_context::<TabsContext>()
        .map(|c| c.orientation)
        .unwrap_or(Orientation::Horizontal);

    let list_style = TypedStyle::new()
        .flex_direction(match orientation {
            Orientation::Horizontal => FlexDirection::Row,
            Orientation::Vertical => FlexDirection::Column,
        })
        .gap(theme.spacing.xs)
        .padding(theme.spacing.xs)
        .surface(TypedSurfaceStyle {
            background: theme.surface.subtle,
            border_width: 1.0,
            border_color: theme.border.default,
            border_radius: theme.radius.md,
            opacity: 1.0,
            box_shadow: None,
        })
        .merge_opt(nve_style.as_ref());

    rsx! {
        view {
            test_id: test_id,
            accessibility_role: "tablist",
            accessibility_label: accessibility_label,
            nve_style: list_style,
            {children}
        }
    }
}

/// Individual tab button (`accessibilityRole="tab"`, `aria-selected`, `aria-controls`,
/// roving `tabIndex`, and arrow/`Home`/`End` keyboard navigation).
#[component]
pub fn TabTrigger(
    value: String,
    #[props(default)] disabled: bool,
    #[props(default)] test_id: Option<String>,
    #[props(default)] nve_style: Option<TypedStyle>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let mut focused = use_signal(|| false);
    let ctx = try_consume_context::<TabsContext>();

    if let Some(ref c) = ctx {
        c.register_tab(&value, disabled);
    }

    let is_selected = ctx
        .as_ref()
        .is_some_and(|c| *c.active.read() == value);
    let controls_id = ctx
        .as_ref()
        .map(|c| c.panel_id(&value))
        .unwrap_or_else(|| format!("tabs-panel-{value}"));
    let tab_id_default = ctx
        .as_ref()
        .map(|c| c.tab_id(&value))
        .unwrap_or_else(|| format!("tabs-tab-{value}"));
    let resolved_test_id = test_id.unwrap_or(tab_id_default);

    let ControlColors { bg, fg, border_width: bw, border: bc } =
        tab_colors(&theme, is_selected, focused() && !disabled);

    let trigger_style = TypedStyle::new()
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .surface(TypedSurfaceStyle {
            background: bg,
            border_width: bw,
            border_color: bc,
            border_radius: theme.radius.sm,
            opacity: if disabled { 0.45 } else { 1.0 },
            box_shadow: None,
        })
        .merge_opt(nve_style.as_ref());

    let text_style = TypedStyle::new().text_style(TypedTextStyle {
        color: fg,
        font_size: theme.typography.ui.size,
        font_weight: if is_selected { 600 } else { 500 },
        text_align: TextAlign::Center,
        opacity: 1.0,
    });

    let aria_selected = if is_selected { "true" } else { "false" };
    let tab_index = if is_selected && !disabled { 0 } else { -1 };

    let ctx_press = ctx.clone();
    let val_press = value.clone();
    let ctx_key = ctx;
    let val_key = value;

    rsx! {
        pressable {
            test_id: "{resolved_test_id}",
            accessibility_role: "tab",
            aria_selected: "{aria_selected}",
            aria_controls: "{controls_id}",
            disabled: disabled,
            tab_index: tab_index,
            nve_style: trigger_style,
            onfocus: move |_| focused.set(true),
            onblur: move |_| focused.set(false),
            onpress: move |_| {
                if !disabled && let Some(mut c) = ctx_press.clone() {
                    c.select(val_press.clone());
                }
            },
            onkeydown: move |ev: KeyEvent| {
                let data = ev.data();
                let key = data.key.as_str();
                if let Some(mut c) = ctx_key.clone() {
                    match key {
                        "Enter" | " " | "Space" if !disabled => c.select(val_key.clone()),
                        _ => c.navigate_key(key),
                    }
                }
            },
            text {
                nve_style: text_style,
                {children}
            }
        }
    }
}

/// Content panel paired with a [`TabTrigger`] (`accessibilityRole="tabpanel"`, `aria-labelledby`).
#[component]
pub fn TabPanel(
    value: String,
    #[props(default)] test_id: Option<String>,
    #[props(default)] nve_style: Option<TypedStyle>,
    children: Element,
) -> Element {
    let theme = use_theme();
    let ctx = try_consume_context::<TabsContext>();
    let is_active = ctx
        .as_ref()
        .is_some_and(|c| *c.active.read() == value);

    if !is_active {
        return rsx! {};
    }

    let labelled_by = ctx
        .as_ref()
        .map(|c| c.tab_id(&value))
        .unwrap_or_else(|| format!("tabs-tab-{value}"));
    let panel_id = test_id.unwrap_or_else(|| {
        ctx.as_ref()
            .map(|c| c.panel_id(&value))
            .unwrap_or_else(|| format!("tabs-panel-{value}"))
    });

    let panel_style = TypedStyle::new()
        .flex_col()
        .padding(theme.spacing.md)
        .gap(theme.spacing.sm)
        .surface(TypedSurfaceStyle {
            background: theme.surface.base,
            border_width: 1.0,
            border_color: theme.border.default,
            border_radius: theme.radius.md,
            opacity: 1.0,
            box_shadow: None,
        })
        .merge_opt(nve_style.as_ref());

    rsx! {
        view {
            test_id: "{panel_id}",
            accessibility_role: "tabpanel",
            aria_labelledby: "{labelled_by}",
            tab_index: 0,
            nve_style: panel_style,
            {children}
        }
    }
}
