//! Responsive navigation chrome for **Aether Studio Workspace**:
//! - Mobile portrait compact top bar + collapsible navigation drawer (`SizeClass::Compact`)
//! - Desktop fixed left sidebar (`SizeClass::Medium` / `SizeClass::Expanded`)
//! - Synchronized `nve-router` Back/Forward route bar with `nve-clipboard` deep-link URL copy
//!
//! Subscriptions stay at the leaves: the pane shells read only the style; the tab list reads
//! `nav` + `counts`, the Studio toggle `chrome`, and the route bar `route`, so typing, scrolling,
//! creating content, or toggling the panel never re-renders an unrelated pane.

use nve_dioxus::prelude::*;
use nve_router::Routable;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Separator,
};

use crate::state::{ActiveView, HOST_SURFACES};
use crate::theme::badge_min_w;
use crate::ui::store::{NavCounts, use_studio};
use crate::ui::style::{
    StudioStyle, StudioStyleExt, badge_dims, fixed_dims, min_w_dims, row_between, row_center,
    text_style, use_studio_style,
};

struct NavItem {
    view: ActiveView,
    test_id: &'static str,
    label: &'static str,
    count: String,
}

fn nav_items(c: &NavCounts) -> [NavItem; 8] {
    let item = |view, test_id, label, count: String| NavItem {
        view,
        test_id,
        label,
        count,
    };
    [
        item(ActiveView::Board, "nav-board", "Board", c.tasks.to_string()),
        item(
            ActiveView::ChannelEngineering,
            "nav-engineering",
            "# engineering",
            c.eng_messages.to_string(),
        ),
        item(
            ActiveView::ChannelDesign,
            "nav-design",
            "# design-sync",
            c.design_messages.to_string(),
        ),
        item(ActiveView::Docs, "nav-docs", "Docs", c.docs.to_string()),
        item(
            ActiveView::Workbench,
            "nav-workbench",
            "Workbench",
            HOST_SURFACES.len().to_string(),
        ),
        item(
            ActiveView::Stream100k,
            "nav-stream",
            "100k Stream",
            "100k".into(),
        ),
        item(
            ActiveView::Islands,
            "nav-islands",
            "Islands",
            c.island_cards.to_string(),
        ),
        item(
            ActiveView::Metrics,
            "nav-metrics",
            "Metrics",
            c.stress_rows.to_string(),
        ),
    ]
}

fn count_badge_style(s: &StudioStyle, label: &str) -> TypedStyle {
    s.compact_control(
        badge_dims(badge_min_w(label) as f32, 18.0),
        4,
        s.m.pill_radius,
        None,
    )
}

/// One navigation tab (sidebar or mobile drawer).
#[component]
fn NavTab(
    view: ActiveView,
    test_id: &'static str,
    label: &'static str,
    count: String,
    active: bool,
    padding: u32,
) -> Element {
    nve_dioxus::render_probe::record_render("NavTab");
    let store = use_studio();
    let s = use_studio_style();
    let bvar = if active {
        BadgeVariant::Accent
    } else {
        BadgeVariant::Neutral
    };
    let badge_style = count_badge_style(&s, &count);
    rsx! {
        pressable {
            test_id: "{test_id}",
            accessibility_role: "tab",
            aria_selected: if active { "true" } else { "false" },
            nve_style: row_between()
                .height_px(28.0)
                .padding_x(padding as f32)
                .bg(s.control_bg(active))
                .radius(s.m.button_radius as f32)
                .outline(s.control(active)),
            onpress: move |_| store.update(|st| st.select_view(view)),
            text { nve_style: text_style(s.control_fg(active), 11.0), "{label}" }
            Badge { label: count, variant: bvar, nve_style: Some(badge_style) }
        }
    }
}

/// The view tabs with their badge counts: the only chrome part subscribed to `counts`.
#[component]
fn NavTabList(padding: u32) -> Element {
    nve_dioxus::render_probe::record_render("NavTabList");
    let store = use_studio();
    let active_view = store.nav.read().active_view;
    let items = nav_items(&store.counts.read().counts);
    rsx! {
        for item in items {
            NavTab {
                key: "{item.test_id}",
                view: item.view,
                test_id: item.test_id,
                label: item.label,
                active: active_view == item.view,
                count: item.count,
                padding,
            }
        }
    }
}

/// Mobile portrait top bar plus its collapsible navigation drawer.
#[component]
pub(crate) fn MobileChrome() -> Element {
    nve_dioxus::render_probe::record_render("MobileChrome");
    let store = use_studio();
    let s = use_studio_style();
    let nav = store.nav.read();
    let (nav_open, studio_open) = (nav.mobile_nav_open, nav.mobile_studio_open);
    drop(nav);

    let pill = |w: f32, active: Option<bool>| {
        s.compact_control(min_w_dims(w, 26.0), 8, s.m.pill_radius, active)
            .white_space(WhiteSpace::NoWrap)
    };
    let mnav_lbl = if nav_open { "Close" } else { "Menu" };
    let mstudio_lbl = if studio_open {
        "Studio ON"
    } else {
        "Studio OFF"
    };

    rsx! {
        view {
            test_id: "mobile-top-bar",
            nve_style: row_between()
                .height_px(42.0)
                .padding_x(10.0)
                .bg(s.t.surface.base)
                .outline(s.panel()),
            view {
                nve_style: row_center(8.0),
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("mobile-toggle-nav".into()),
                    nve_style: Some(pill(52.0, Some(nav_open))),
                    on_press: move |_| store.update(|st| st.toggle_mobile_nav()),
                    "{mnav_lbl}"
                }
                text { nve_style: text_style(s.t.text.link, 13.0), "AETHER" }
            }
            view {
                nve_style: row_center(6.0),
                Button {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-new-task".into()),
                    nve_style: Some(pill(58.0, None)),
                    on_press: move |_| store.update(|st| st.create_new_task()),
                    "+ Task"
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-toggle-studio".into()),
                    nve_style: Some(pill(72.0, Some(studio_open))),
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
                nve_style: Some(
                    TypedStyle::new()
                        .width_pct(100.0)
                        .flex_shrink(0.0)
                        .radius(0.0)
                        .outline(s.card()),
                ),
                NavTabList { padding: 10 }
            }
        }
    }
}

/// The sidebar's docked-Studio toggle: the only sidebar part subscribed to `chrome`.
#[component]
fn StudioPanelToggle() -> Element {
    nve_dioxus::render_probe::record_render("StudioPanelToggle");
    let store = use_studio();
    let s = use_studio_style();
    let studio_open = store.chrome.read().panel_open;
    let studio_badge_text = if studio_open { "ON" } else { "OFF" };
    let studio_badge_variant = if studio_open {
        BadgeVariant::Accent
    } else {
        BadgeVariant::Neutral
    };
    let studio_badge_style = count_badge_style(&s, studio_badge_text);
    rsx! {
        pressable {
            test_id: "btn-toggle-studio",
            nve_style: row_between()
                .height_px(30.0)
                .padding_x(9.0)
                .bg(s.control_bg(studio_open))
                .radius(s.m.button_radius as f32)
                .outline(s.control(studio_open)),
            onpress: move |_| store.update(|st| st.toggle_studio_panel()),
            text { nve_style: text_style(s.t.text.primary, 11.0), "NVE Studio" }
            Badge {
                label: studio_badge_text.to_string(),
                variant: studio_badge_variant,
                nve_style: Some(studio_badge_style),
            }
        }
    }
}

/// Desktop fixed left sidebar (style-only shell around the subscribed leaves).
#[component]
pub(crate) fn DesktopSidebar() -> Element {
    nve_dioxus::render_probe::record_render("DesktopSidebar");
    let store = use_studio();
    let s = use_studio_style();
    let new_task_style = s
        .compact_control(
            TypedStyle::new()
                .width_pct(100.0)
                .height_px(30.0)
                .min_height_px(30.0)
                .flex_shrink(0.0),
            6,
            s.m.button_radius,
            None,
        )
        .outline(s.row());

    rsx! {
        view {
            test_id: "desktop-sidebar",
            nve_style: TypedStyle::new()
                .width_px(172.0)
                .height_pct(100.0)
                .flex_shrink(0.0)
                .flex_col()
                .padding(12.0)
                .gap(8.0)
                .bg(s.t.surface.base)
                .outline(s.panel()),

            Card {
                variant: CardVariant::Subtle,
                padding: Some(8.0),
                gap: Some(2.0),
                nve_style: Some(s.card_surface(8, 2, s.t.surface.sunken, s.m.button_radius)),
                text { nve_style: text_style(s.t.text.link, 15.0), "AETHER" }
                text { nve_style: text_style(s.t.text.tertiary, 10.0), "LOCAL-FIRST | NVE" }
            }

            view {
                accessibility_role: "tablist",
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_shrink(0.0)
                    .flex_col()
                    .gap(5.0)
                    .outline(s.row()),
                NavTabList { padding: 9 }
            }

            view { nve_style: TypedStyle::new().width_pct(100.0).flex_1() }

            Separator {}

            view {
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_shrink(0.0)
                    .flex_col()
                    .gap(6.0)
                    .outline(s.row()),
                Button {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-new-task".into()),
                    nve_style: Some(new_task_style),
                    on_press: move |_| store.update(|st| st.create_new_task()),
                    "+ New Task"
                }
                StudioPanelToggle {}
            }
        }
    }
}

/// `nve-router` Back/Forward bar with the current path and a deep-link copy button.
#[component]
pub(crate) fn RouteNavBar(compact_header: bool) -> Element {
    nve_dioxus::render_probe::record_render("RouteNavBar");
    let store = use_studio();
    let s = use_studio_style();
    let route = store.route.read();
    let (can_back, can_forward) = (route.can_back, route.can_forward);
    let route_bar_text = match &route.clipboard_notice {
        Some(notice) if !compact_header => format!("{} | {notice}", route.route.to_path()),
        _ => route.route.to_path(),
    };
    drop(route);

    let card = s.card();
    let step_style = |enabled: bool| {
        s.compact_control(fixed_dims(22.0, 20.0), 0, s.t.radius.sm as u32, None)
            .bg(s.t.surface.subtle)
            .border(1.0, card.color)
            .opacity(if enabled { 1.0 } else { 0.35 })
    };
    let copy_url_lbl = if compact_header { "URL" } else { "Copy URL" };
    let copy_url_w = if compact_header { 34.0 } else { 58.0 };
    let copy_url_style = s
        .compact_control(min_w_dims(copy_url_w, 20.0), 6, s.m.pill_radius, None)
        .white_space(WhiteSpace::NoWrap)
        .bg(s.t.surface.subtle)
        .outline(s.control(true));

    rsx! {
        view {
            test_id: "route-nav-bar",
            nve_style: row_center(4.0)
                .width_pct(100.0)
                .flex_shrink(0.0)
                .padding_xy(6.0, 3.0)
                .bg(s.t.surface.sunken)
                .radius(s.m.button_radius as f32)
                .outline(card),
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("nav-back".into()),
                accessibility_label: Some("Navigate back".into()),
                nve_style: Some(step_style(can_back)),
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
                nve_style: Some(step_style(can_forward)),
                on_press: move |_| store.update(|st| {
                    let _ = st.navigate_forward();
                }),
                ">"
            }
            text {
                test_id: "route-current-path",
                nve_style: text_style(s.t.text.secondary, 10.0).flex_1(),
                "{route_bar_text}"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("btn-copy-route-url".into()),
                accessibility_label: Some("Copy deep-link route URL".into()),
                nve_style: Some(copy_url_style),
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
