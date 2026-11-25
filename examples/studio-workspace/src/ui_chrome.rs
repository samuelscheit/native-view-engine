//! Responsive navigation chrome for **Aether Studio Workspace**:
//! - Mobile portrait compact top bar + collapsible navigation drawer (`SizeClass::Compact`)
//! - Desktop fixed left sidebar (`SizeClass::Medium` / `SizeClass::Expanded`)
//! - Synchronized `nve-router` Back/Forward route bar with `nve-sdk` deep-link URL copy

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_ui::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Separator};

use crate::sdk::StudioHostServices;
use crate::state::{ActiveView, WorkspaceSnapshot, WorkspaceState};
use crate::theme::{
    badge_min_w, card_border, control_border, panel_border, row_border, LayoutMetrics, CARD_ALT_BG,
    CONTROL_BG, PANEL_BG, STRETCH_COL, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY, XRAY_ROW,
};
use crate::ui::mutate_state;

pub(crate) type NavItem = (ActiveView, &'static str, &'static str, String);

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_mobile_top_bar_and_nav(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    current: &WorkspaceSnapshot,
    nav_items: &[NavItem],
    accent_hex: &'static str,
    accent_tint: &'static str,
    xray: bool,
    m: LayoutMetrics,
) -> Element {
    let (panel_bw, panel_bc) = panel_border(xray);
    let (card_bw, card_bc) = card_border(xray);
    let active_view = current.active_view;

    let studio_open_active = current.mobile_studio_open;
    let (studio_btn_bw, studio_btn_bc) = control_border(xray, studio_open_active, accent_hex);
    let studio_btn_bg = if studio_open_active {
        accent_tint
    } else {
        CONTROL_BG
    };
    let studio_badge_text = if studio_open_active { "ON" } else { "OFF" };

    let (mnav_bw, mnav_bc) = control_border(xray, current.mobile_nav_open, accent_hex);
    let mnav_bg = if current.mobile_nav_open {
        accent_tint
    } else {
        CONTROL_BG
    };
    let mnav_lbl = if current.mobile_nav_open { "Close" } else { "Menu" };

    let mnav_btn_style = format!(
        "width: 52px; height: 26px; min-height: 26px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {mnav_bg}; border-radius: {}px; border-width: {mnav_bw}px; border-color: {mnav_bc};",
        m.pill_radius
    );
    let mtask_btn_style = format!(
        "width: 58px; height: 26px; min-height: 26px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; border-radius: {}px;",
        m.pill_radius
    );
    let mstudio_btn_style = format!(
        "width: 72px; height: 26px; min-height: 26px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {studio_btn_bg}; border-radius: {}px; border-width: {studio_btn_bw}px; border-color: {studio_btn_bc};",
        m.pill_radius
    );
    let mstudio_lbl = format!("Studio {studio_badge_text}");

    let s_toggle_nav = Arc::clone(store);
    let s_new_task = Arc::clone(store);
    let s_toggle_studio = Arc::clone(store);

    rsx! {
        view {
            test_id: "mobile-top-bar",
            style: "width: 100%; height: 42px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {PANEL_BG}; border-width: {panel_bw}px; border-color: {panel_bc};",
            view {
                style: "flex-direction: row; align-items: center; gap: 8px;",
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("mobile-toggle-nav".into()),
                    style: Some(mnav_btn_style),
                    on_press: move |_| mutate_state(&s_toggle_nav, version, |st| st.toggle_mobile_nav()),
                    "{mnav_lbl}"
                }
                text {
                    style: "color: {accent_hex}; font-size: 13px;",
                    "AETHER"
                }
            }
            view {
                style: "flex-direction: row; align-items: center; gap: 6px;",
                Button {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-new-task".into()),
                    style: Some(mtask_btn_style),
                    on_press: move |_| mutate_state(&s_new_task, version, |st| st.create_new_task()),
                    "+ Task"
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-toggle-studio".into()),
                    style: Some(mstudio_btn_style),
                    on_press: move |_| mutate_state(&s_toggle_studio, version, |st| st.toggle_studio_panel()),
                    "{mstudio_lbl}"
                }
            }
        }

        if current.mobile_nav_open {
            Card {
                variant: CardVariant::Outlined,
                test_id: Some("mobile-nav-drawer".into()),
                padding: Some(8.0),
                gap: Some(5.0),
                style: Some(format!("width: 100%; flex-shrink: 0; border-radius: 0px; border-width: {card_bw}px; border-color: {card_bc};")),
                for (view_kind, tid, label, count) in nav_items.iter() {
                    {
                        let vk = *view_kind;
                        let is_active = active_view == vk;
                        let (bw, bc) = control_border(xray, is_active, accent_hex);
                        let bg = if is_active { accent_tint } else { CONTROL_BG };
                        let fg = if is_active { TEXT_PRIMARY } else { TEXT_SECONDARY };
                        let bvar = if is_active { BadgeVariant::Accent } else { BadgeVariant::Neutral };
                        let bstyle = format!(
                            "min-width: {}px; height: 18px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; border-radius: {}px;",
                            badge_min_w(count),
                            m.pill_radius
                        );
                        let s_nav = Arc::clone(store);
                        rsx! {
                            pressable {
                                key: "m-{tid}",
                                test_id: "{tid}",
                                style: "width: 100%; height: 28px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {bg}; border-radius: {m.button_radius}px; border-width: {bw}px; border-color: {bc};",
                                onpress: move |_| mutate_state(&s_nav, version, |st| st.select_view(vk)),
                                text { style: "color: {fg}; font-size: 11px;", "{label}" }
                                Badge {
                                    label: count.clone(),
                                    variant: bvar,
                                    style: Some(bstyle),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_desktop_sidebar(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    current: &WorkspaceSnapshot,
    nav_items: &[NavItem],
    accent_hex: &'static str,
    accent_tint: &'static str,
    xray: bool,
    m: LayoutMetrics,
) -> Element {
    let (panel_bw, panel_bc) = panel_border(xray);
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);
    let active_view = current.active_view;

    let sidebar_style = format!(
        "width: 172px; height: 100%; flex-shrink: 0; flex-direction: column; padding: 12px; gap: 8px; background-color: {PANEL_BG}; border-width: {panel_bw}px; border-color: {panel_bc};"
    );
    let brand_card_style = format!(
        "width: 100%; flex-shrink: 0; padding: 8px; gap: 2px; background-color: {CARD_ALT_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );
    let new_task_style = format!(
        "width: 100%; height: 30px; min-height: 30px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 6px; padding-vertical: 0px; border-radius: {}px; border-width: {}px; border-color: {};",
        m.button_radius,
        if xray { 1 } else { 0 },
        if xray { XRAY_ROW } else { "transparent" }
    );

    let studio_open_active = current.studio.panel_open;
    let (studio_btn_bw, studio_btn_bc) = control_border(xray, studio_open_active, accent_hex);
    let studio_btn_bg = if studio_open_active {
        accent_tint
    } else {
        CONTROL_BG
    };
    let studio_toggle_style = format!(
        "width: 100%; height: 30px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 9px; background-color: {studio_btn_bg}; border-radius: {}px; border-width: {studio_btn_bw}px; border-color: {studio_btn_bc};",
        m.button_radius
    );
    let studio_badge_text = if studio_open_active { "ON" } else { "OFF" };
    let studio_badge_variant = if studio_open_active {
        BadgeVariant::Accent
    } else {
        BadgeVariant::Neutral
    };
    let studio_badge_style = format!(
        "min-width: {}px; height: 18px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; border-radius: {}px;",
        badge_min_w(studio_badge_text),
        m.pill_radius
    );

    let s_new_task = Arc::clone(store);
    let s_toggle_studio = Arc::clone(store);

    rsx! {
        view {
            test_id: "desktop-sidebar",
            style: "{sidebar_style}",

            Card {
                variant: CardVariant::Subtle,
                padding: Some(8.0),
                gap: Some(2.0),
                style: Some(brand_card_style),
                text {
                    style: "color: {accent_hex}; font-size: 15px;",
                    "AETHER"
                }
                text {
                    style: "color: {TEXT_MUTED}; font-size: 10px;",
                    "LOCAL-FIRST | NVE"
                }
            }

            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 5px; border-width: {row_bw}px; border-color: {row_bc};",
                for (view_kind, tid, label, count) in nav_items.iter() {
                    {
                        let vk = *view_kind;
                        let is_active = active_view == vk;
                        let (bw, bc) = control_border(xray, is_active, accent_hex);
                        let bg = if is_active { accent_tint } else { CONTROL_BG };
                        let fg = if is_active { TEXT_PRIMARY } else { TEXT_SECONDARY };
                        let bvar = if is_active { BadgeVariant::Accent } else { BadgeVariant::Neutral };
                        let tab_style = format!(
                            "width: 100%; height: 28px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 9px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                            m.button_radius
                        );
                        let badge_style = format!(
                            "min-width: {}px; height: 18px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; border-radius: {}px;",
                            badge_min_w(count),
                            m.pill_radius
                        );
                        let s_nav = Arc::clone(store);
                        rsx! {
                            pressable {
                                key: "{tid}",
                                test_id: "{tid}",
                                style: "{tab_style}",
                                onpress: move |_| mutate_state(&s_nav, version, |st| st.select_view(vk)),
                                text {
                                    style: "color: {fg}; font-size: 11px;",
                                    "{label}"
                                }
                                Badge {
                                    label: count.clone(),
                                    variant: bvar,
                                    style: Some(badge_style),
                                }
                            }
                        }
                    }
                }
            }

            view { style: "width: 100%; flex: 1;" }

            Separator {}

            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 6px; border-width: {row_bw}px; border-color: {row_bc};",
                Button {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    test_id: Some("btn-new-task".into()),
                    style: Some(new_task_style),
                    on_press: move |_| mutate_state(&s_new_task, version, |st| st.create_new_task()),
                    "+ New Task"
                }
                pressable {
                    test_id: "btn-toggle-studio",
                    style: "{studio_toggle_style}",
                    onpress: move |_| mutate_state(&s_toggle_studio, version, |st| st.toggle_studio_panel()),
                    text {
                        style: "color: {TEXT_PRIMARY}; font-size: 11px;",
                        "NVE Studio"
                    }
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_route_nav_bar(
    store: &Arc<Mutex<WorkspaceState>>,
    host_services: &Arc<StudioHostServices>,
    version: Signal<u64>,
    route_path: &str,
    clipboard_notice: Option<&str>,
    can_nav_back: bool,
    can_nav_forward: bool,
    compact_header: bool,
    accent_hex: &'static str,
    xray: bool,
    m: LayoutMetrics,
) -> Element {
    let (card_bw, card_bc) = card_border(xray);
    let back_op = if can_nav_back { "1.0" } else { "0.35" };
    let fwd_op = if can_nav_forward { "1.0" } else { "0.35" };
    let nav_back_style = format!(
        "width: 22px; height: 20px; min-height: 20px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 0px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: 4px; border-width: 1px; border-color: {card_bc}; opacity: {back_op};"
    );
    let nav_fwd_style = format!(
        "width: 22px; height: 20px; min-height: 20px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 0px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: 4px; border-width: 1px; border-color: {card_bc}; opacity: {fwd_op};"
    );
    let copy_url_lbl = if compact_header { "URL" } else { "Copy URL" };
    let copy_url_w = if compact_header { 34 } else { 58 };
    let copy_url_style = format!(
        "width: {copy_url_w}px; height: 20px; min-height: 20px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        m.pill_radius
    );
    let route_bar_text = match clipboard_notice {
        Some(notice) if !compact_header => format!("{route_path} | {notice}"),
        _ => route_path.to_owned(),
    };

    let s_nav_back = Arc::clone(store);
    let s_nav_fwd = Arc::clone(store);
    let s_copy_url = Arc::clone(store);
    let hs_copy_url = Arc::clone(host_services);

    rsx! {
        view {
            test_id: "route-nav-bar",
            style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 4px; padding-horizontal: 6px; padding-vertical: 3px; background-color: {CARD_ALT_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("nav-back".into()),
                accessibility_label: Some("Navigate back".into()),
                style: Some(nav_back_style),
                on_press: move |_| mutate_state(&s_nav_back, version, |st| {
                    let _ = st.navigate_back();
                }),
                "<"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("nav-forward".into()),
                accessibility_label: Some("Navigate forward".into()),
                style: Some(nav_fwd_style),
                on_press: move |_| mutate_state(&s_nav_fwd, version, |st| {
                    let _ = st.navigate_forward();
                }),
                ">"
            }
            text {
                test_id: "route-current-path",
                style: "flex: 1; color: {TEXT_SECONDARY}; font-size: 10px;",
                "{route_bar_text}"
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("btn-copy-route-url".into()),
                accessibility_label: Some("Copy deep-link route URL".into()),
                style: Some(copy_url_style),
                on_press: move |_| mutate_state(&s_copy_url, version, |st| {
                    let _ = st.copy_current_route_url(&hs_copy_url);
                }),
                "{copy_url_lbl}"
            }
        }
    }
}
