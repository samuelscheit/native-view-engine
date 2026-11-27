//! Responsive navigation chrome for **Aether Studio Workspace**:
//! - Mobile portrait compact top bar + collapsible navigation drawer (`SizeClass::Compact`)
//! - Desktop fixed left sidebar (`SizeClass::Medium` / `SizeClass::Expanded`)
//! - Synchronized `nve-router` Back/Forward route bar with `nve-clipboard` deep-link URL copy
//!
//! Each is a component subscribed to the `nav` domain only (plus `studio` for the panel toggle).

use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, ColorHex, Separator,
};

use crate::state::{ActiveView, HOST_SURFACES};
use crate::theme::{badge_min_w, STRETCH_COL};
use crate::ui::store::{use_studio, NavCounts};
use crate::ui::style::{use_studio_style, StudioStyle};

struct NavItem {
    view: ActiveView,
    test_id: &'static str,
    label: &'static str,
    count: String,
}

fn nav_items(c: &NavCounts) -> [NavItem; 8] {
    let item = |view, test_id, label, count: String| NavItem { view, test_id, label, count };
    [
        item(ActiveView::Board, "nav-board", "Board", c.tasks.to_string()),
        item(ActiveView::ChannelEngineering, "nav-engineering", "# engineering", c.eng_messages.to_string()),
        item(ActiveView::ChannelDesign, "nav-design", "# design-sync", c.design_messages.to_string()),
        item(ActiveView::Docs, "nav-docs", "Docs", c.docs.to_string()),
        item(ActiveView::Workbench, "nav-workbench", "Workbench", HOST_SURFACES.len().to_string()),
        item(ActiveView::Stream100k, "nav-stream", "100k Stream", "100k".into()),
        item(ActiveView::Islands, "nav-islands", "Islands", c.island_cards.to_string()),
        item(ActiveView::Metrics, "nav-metrics", "Metrics", c.stress_rows.to_string()),
    ]
}

fn count_badge_style(s: &StudioStyle, label: &str) -> String {
    format!(
        "min-width: {}px; height: 18px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; border-radius: {}px;",
        badge_min_w(label),
        s.m.pill_radius
    )
}

/// One navigation tab (sidebar or mobile drawer).
#[component]
fn NavTab(view: ActiveView, test_id: &'static str, label: &'static str, count: String, active: bool, padding: u32) -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let bvar = if active { BadgeVariant::Accent } else { BadgeVariant::Neutral };
    let badge_style = count_badge_style(&s, &count);
    rsx! {
        pressable {
            test_id: "{test_id}",
            style: "width: 100%; height: 28px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: {padding}px; background-color: {s.control_bg(active)}; border-radius: {s.m.button_radius}px; {s.control(active)}",
            onpress: move |_| store.update(|st| st.select_view(view)),
            text { style: "color: {s.control_fg(active)}; font-size: 11px;", "{label}" }
            Badge { label: count, variant: bvar, style: Some(badge_style) }
        }
    }
}

/// Mobile portrait top bar plus its collapsible navigation drawer.
#[component]
pub(crate) fn MobileChrome() -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let nav = store.nav.read();
    let (nav_open, studio_open, active_view) = (nav.mobile_nav_open, nav.mobile_studio_open, nav.active_view);
    let items = nav_items(&nav.counts);
    drop(nav);

    let pill = |w: u32, active: Option<bool>| {
        let paint = active.map_or_else(String::new, |a| format!("background-color: {}; {}", s.control_bg(a), s.control(a)));
        format!(
            "width: {w}px; height: 26px; min-height: 26px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; border-radius: {}px; {paint}",
            s.m.pill_radius
        )
    };
    let mnav_lbl = if nav_open { "Close" } else { "Menu" };
    let mstudio_lbl = if studio_open { "Studio ON" } else { "Studio OFF" };

    rsx! {
        view {
            test_id: "mobile-top-bar",
            style: "width: 100%; height: 42px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {s.t.surface.base.hex()}; {s.panel()}",
            view {
                style: "flex-direction: row; align-items: center; gap: 8px;",
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("mobile-toggle-nav".into()),
                    style: Some(pill(52, Some(nav_open))),
                    on_press: move |_| store.update(|st| st.toggle_mobile_nav()),
                    "{mnav_lbl}"
                }
                text { style: "color: {s.t.text.link.hex()}; font-size: 13px;", "AETHER" }
            }
            view {
                style: "flex-direction: row; align-items: center; gap: 6px;",
                Button {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-new-task".into()),
                    style: Some(pill(58, None)),
                    on_press: move |_| store.update(|st| st.create_new_task()),
                    "+ Task"
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-toggle-studio".into()),
                    style: Some(pill(72, Some(studio_open))),
                    on_press: move |_| store.update(|st| st.toggle_studio_panel()),
                    "{mstudio_lbl}"
                }
            }
        }

        if nav_open {
            Card {
                variant: CardVariant::Outlined,
                test_id: Some("mobile-nav-drawer".into()),
                padding: Some(8.0),
                gap: Some(5.0),
                style: Some(format!("width: 100%; flex-shrink: 0; border-radius: 0px; {}", s.card())),
                for item in items {
                    NavTab {
                        key: "m-{item.test_id}",
                        view: item.view,
                        test_id: item.test_id,
                        label: item.label,
                        active: active_view == item.view,
                        count: item.count,
                        padding: 10,
                    }
                }
            }
        }
    }
}

/// Desktop fixed left sidebar.
#[component]
pub(crate) fn DesktopSidebar() -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let nav = store.nav.read();
    let active_view = nav.active_view;
    let items = nav_items(&nav.counts);
    drop(nav);
    let studio_open = store.studio.read().config.panel_open;

    let new_task_style = format!(
        "width: 100%; height: 30px; min-height: 30px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 6px; padding-vertical: 0px; border-radius: {}px; {}",
        s.m.button_radius,
        s.row()
    );
    let studio_badge_text = if studio_open { "ON" } else { "OFF" };
    let studio_badge_variant = if studio_open { BadgeVariant::Accent } else { BadgeVariant::Neutral };
    let studio_badge_style = count_badge_style(&s, studio_badge_text);

    rsx! {
        view {
            test_id: "desktop-sidebar",
            style: "width: 172px; height: 100%; flex-shrink: 0; flex-direction: column; padding: 12px; gap: 8px; background-color: {s.t.surface.base.hex()}; {s.panel()}",

            Card {
                variant: CardVariant::Subtle,
                padding: Some(8.0),
                gap: Some(2.0),
                style: Some(format!(
                    "width: 100%; flex-shrink: 0; padding: 8px; gap: 2px; background-color: {}; border-radius: {}px; {}",
                    s.t.surface.sunken.hex(),
                    s.m.button_radius,
                    s.card()
                )),
                text { style: "color: {s.t.text.link.hex()}; font-size: 15px;", "AETHER" }
                text { style: "color: {s.t.text.tertiary.hex()}; font-size: 10px;", "LOCAL-FIRST | NVE" }
            }

            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 5px; {s.row()}",
                for item in items {
                    NavTab {
                        key: "{item.test_id}",
                        view: item.view,
                        test_id: item.test_id,
                        label: item.label,
                        active: active_view == item.view,
                        count: item.count,
                        padding: 9,
                    }
                }
            }

            view { style: "width: 100%; flex: 1;" }

            Separator {}

            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 6px; {s.row()}",
                Button {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-new-task".into()),
                    style: Some(new_task_style),
                    on_press: move |_| store.update(|st| st.create_new_task()),
                    "+ New Task"
                }
                pressable {
                    test_id: "btn-toggle-studio",
                    style: "width: 100%; height: 30px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 9px; background-color: {s.control_bg(studio_open)}; border-radius: {s.m.button_radius}px; {s.control(studio_open)}",
                    onpress: move |_| store.update(|st| st.toggle_studio_panel()),
                    text { style: "color: {s.t.text.primary.hex()}; font-size: 11px;", "NVE Studio" }
                    Badge {
                        label: studio_badge_text.to_string(),
                        variant: studio_badge_variant,
                        style: Some(studio_badge_style),
                    }
                }
            }
        }
    }
}

/// `nve-router` Back/Forward bar with the current path and a deep-link copy button.
#[component]
pub(crate) fn RouteNavBar(compact_header: bool) -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let nav = store.nav.read();
    let (can_back, can_forward) = (nav.can_back, nav.can_forward);
    let route_bar_text = match &nav.clipboard_notice {
        Some(notice) if !compact_header => format!("{} | {notice}", nav.route_path),
        _ => nav.route_path.clone(),
    };
    drop(nav);

    let card = s.card();
    let step_style = |enabled: bool| {
        format!(
            "width: 22px; height: 20px; min-height: 20px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 0px; padding-vertical: 0px; background-color: {}; border-radius: {}px; border-width: 1px; border-color: {}; opacity: {};",
            s.t.surface.subtle.hex(),
            s.t.radius.sm,
            card.color.hex(),
            if enabled { "1.0" } else { "0.35" }
        )
    };
    let copy_url_lbl = if compact_header { "URL" } else { "Copy URL" };
    let copy_url_w = if compact_header { 34 } else { 58 };
    let copy_url_style = format!(
        "width: {copy_url_w}px; height: 20px; min-height: 20px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {}; border-radius: {}px; {}",
        s.t.surface.subtle.hex(),
        s.m.pill_radius,
        s.control(true)
    );

    rsx! {
        view {
            test_id: "route-nav-bar",
            style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 4px; padding-horizontal: 6px; padding-vertical: 3px; background-color: {s.t.surface.sunken.hex()}; border-radius: {s.m.button_radius}px; {card}",
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("nav-back".into()),
                accessibility_label: Some("Navigate back".into()),
                style: Some(step_style(can_back)),
                on_press: move |_| store.update(|st| {
                    let _ = st.navigate_back();
                }),
                "<"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("nav-forward".into()),
                accessibility_label: Some("Navigate forward".into()),
                style: Some(step_style(can_forward)),
                on_press: move |_| store.update(|st| {
                    let _ = st.navigate_forward();
                }),
                ">"
            }
            text {
                test_id: "route-current-path",
                style: "flex: 1; color: {s.t.text.secondary.hex()}; font-size: 10px;",
                "{route_bar_text}"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("btn-copy-route-url".into()),
                accessibility_label: Some("Copy deep-link route URL".into()),
                style: Some(copy_url_style),
                on_press: move |_| {
                    let services = store.services();
                    store.update(|st| {
                        let _ = st.copy_current_route_url(&services);
                    });
                },
                "{copy_url_lbl}"
            }
        }
    }
}
