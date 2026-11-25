//! Dioxus 0.7 `rsx!` shell for **Aether Workspace** + docked **NVE Studio** meta-workbench.
//!
//! Powered by the shared `nve-ui` component kit (`ThemeProvider`, `Card`, `Button`, `Badge`,
//! `Separator`, `Tabs*`, `Input`, `Dialog`) and automatically adapting between the desktop
//! 3-column workspace (`>= 620pt`) and a responsive single-column mobile portrait shell
//! (`< 620pt`, e.g. iPhone 17 393pt) with collapsible top navigation and NVE Studio drawers.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_ui::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Separator, ThemeProvider};

use crate::state::{ActiveView, ChannelKind, WorkspaceState, HOST_SURFACES};
use crate::theme::{
    badge_min_w, card_border, control_border, panel_border, row_border, studio_ui_theme,
    LayoutMetrics, CANVAS_BG, CARD_ALT_BG, CONTROL_BG, PANEL_BG, STRETCH_COL, TEXT_MUTED,
    TEXT_PRIMARY, TEXT_SECONDARY, XRAY_ROW,
};
use crate::ui_board::render_board_view;
use crate::ui_channels::render_channel_view;
use crate::ui_docs::render_docs_view;
use crate::ui_islands_view::render_islands_view;
use crate::ui_metrics::render_metrics_view;
use crate::ui_palette::{filter_palette_items, render_command_palette};
use crate::ui_stream::{SharedStore, Stream100kPanel};
use crate::ui_studio::render_studio_panel;
use crate::ui_workbench::render_workbench_view;

thread_local! {
    /// Hands a module-owned `Arc<Mutex<WorkspaceState>>` into the `DioxusSurface` root hook
    /// on first mount without any cross-instance global contention.
    static PENDING_STORE: RefCell<Option<Arc<Mutex<WorkspaceState>>>> = const { RefCell::new(None) };
}

pub(crate) fn set_pending_store(store: Arc<Mutex<WorkspaceState>>) {
    PENDING_STORE.with(|slot| {
        *slot.borrow_mut() = Some(store);
    });
}

pub(crate) fn clear_pending_store() {
    PENDING_STORE.with(|slot| {
        slot.borrow_mut().take();
    });
}

fn take_pending_store() -> Option<Arc<Mutex<WorkspaceState>>> {
    PENDING_STORE.with(|slot| slot.borrow_mut().take())
}

pub(crate) fn mutate_state(
    store: &Arc<Mutex<WorkspaceState>>,
    mut version: Signal<u64>,
    f: impl FnOnce(&mut WorkspaceState),
) {
    if let Ok(mut guard) = store.lock() {
        f(&mut guard);
    }
    let next = version().wrapping_add(1);
    version.set(next);
}

#[component]
pub fn StudioWorkspaceApp() -> Element {
    let store = dioxus_core::use_hook(|| {
        take_pending_store().unwrap_or_else(|| Arc::new(Mutex::new(WorkspaceState::default())))
    });
    let version = use_signal(|| 0u64);
    let rev = version();

    // Clone only the active snapshot + lightweight metadata (never the full 64-entry history vec!).
    let (
        current,
        palette,
        sync_telemetry,
        palette_matches,
        is_mobile,
        frame_num,
        snap_idx,
        hist_len,
        can_undo,
        can_redo,
        active_tasks,
        done_tasks,
        total_msgs,
        filtered_tasks,
    ) = {
        let guard = store.lock().unwrap_or_else(|e| e.into_inner());
        let active_view = guard.current.active_view;
        let filtered = if active_view == ActiveView::Board {
            guard.filtered_tasks()
        } else {
            Vec::new()
        };
        let pal_matches = if guard.palette.open {
            filter_palette_items(&guard)
        } else {
            Vec::new()
        };
        (
            guard.current.clone(),
            guard.palette.clone(),
            guard.sync_telemetry.clone(),
            pal_matches,
            guard.is_mobile_portrait(),
            guard.frame_count,
            guard.snapshot_index_1based(),
            guard.history_len(),
            guard.can_undo(),
            guard.can_redo(),
            guard.active_task_count(),
            guard.done_task_count(),
            guard.total_message_count(),
            filtered,
        )
    };

    let studio = &current.studio;
    let ui_theme = studio_ui_theme(studio);
    let accent = studio.accent;
    let accent_hex = accent.hex();
    let accent_tint = accent.tint_bg();
    let xray = studio.xray;
    let layout_engine_str = studio.layout_engine.as_str();
    let viewport_mode_str = if is_mobile {
        "mobile-portrait"
    } else {
        "desktop"
    };
    let m = LayoutMetrics::from_studio(studio);

    let (panel_bw, panel_bc) = panel_border(xray);
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);

    let active_view = current.active_view;
    let total_tasks = current.tasks.len();
    let eng_count = current.eng_messages.len();
    let design_count = current.design_messages.len();
    let docs_count = current.docs.len();
    let hosts_count = HOST_SURFACES.len();
    let islands_count = current.islands.cards.len();
    let stress_rows = current.stress_rows;

    let nav_items: &[(ActiveView, &str, &str, String)] = &[
        (ActiveView::Board, "nav-board", "Board", total_tasks.to_string()),
        (ActiveView::ChannelEngineering, "nav-engineering", "# engineering", eng_count.to_string()),
        (ActiveView::ChannelDesign, "nav-design", "# design-sync", design_count.to_string()),
        (ActiveView::Docs, "nav-docs", "Docs", docs_count.to_string()),
        (ActiveView::Workbench, "nav-workbench", "Workbench", hosts_count.to_string()),
        (ActiveView::Stream100k, "nav-stream", "100k Stream", "100k".into()),
        (ActiveView::Islands, "nav-islands", "Islands", islands_count.to_string()),
        (ActiveView::Metrics, "nav-metrics", "Metrics", stress_rows.to_string()),
    ];

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

    let studio_open_active = if is_mobile {
        current.mobile_studio_open
    } else {
        studio.panel_open
    };
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

    let center_pad = if is_mobile {
        m.main_pad.min(10)
    } else {
        m.main_pad
    };
    let center_style = if is_mobile {
        format!(
            "width: 100%; flex: 1; flex-direction: column; padding: {center_pad}px; gap: {}px; background-color: {CANVAS_BG}; border-width: {panel_bw}px; border-color: {panel_bc};",
            m.main_gap
        )
    } else {
        format!(
            "flex: 1; height: 100%; flex-direction: column; padding: {center_pad}px; gap: {}px; background-color: {CANVAS_BG}; border-width: {panel_bw}px; border-color: {panel_bc};",
            m.main_gap
        )
    };

    let header_card_style = format!(
        "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 8px; padding-vertical: 6px; gap: 6px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );

    let compact_header = current.viewport_width < 760;
    let status_pill_label = if compact_header {
        format!("F#{frame_num} {snap_idx}/{hist_len}")
    } else {
        format!("Frame #{frame_num} | {snap_idx}/{hist_len}")
    };
    let status_pill_style = format!(
        "min-width: {}px; height: 22px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 4px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        badge_min_w(&status_pill_label),
        m.pill_radius
    );
    let (cmdk_bw, cmdk_bc) = control_border(xray, palette.open, accent_hex);
    let cmdk_bg = if palette.open { accent_tint } else { CONTROL_BG };
    let cmdk_btn_style = format!(
        "width: 52px; height: 22px; min-height: 22px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {cmdk_bg}; border-radius: {}px; border-width: {cmdk_bw}px; border-color: {cmdk_bc};",
        m.pill_radius
    );

    let active_channel = match active_view {
        ActiveView::ChannelDesign => Some(ChannelKind::Design),
        ActiveView::ChannelEngineering => Some(ChannelKind::Engineering),
        _ => None,
    };
    let channel_messages = match active_channel {
        Some(ChannelKind::Engineering) => current.eng_messages.as_slice(),
        Some(ChannelKind::Design) => current.design_messages.as_slice(),
        None => &[],
    };
    let channel_draft = match active_channel {
        Some(ChannelKind::Engineering) => current.eng_draft.as_str(),
        Some(ChannelKind::Design) => current.design_draft.as_str(),
        None => "",
    };

    let root_flex_dir = if is_mobile { "column" } else { "row" };

    let s_new_task = Arc::clone(&store);
    let s_new_task_mobile = Arc::clone(&store);
    let s_toggle_studio = Arc::clone(&store);
    let s_toggle_studio_mobile = Arc::clone(&store);
    let s_toggle_nav_mobile = Arc::clone(&store);
    let s_toggle_palette = Arc::clone(&store);

    let (mnav_bw, mnav_bc) = control_border(xray, current.mobile_nav_open, accent_hex);
    let mnav_bg = if current.mobile_nav_open { accent_tint } else { CONTROL_BG };
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

    rsx! {
        ThemeProvider {
            theme: ui_theme,
            view {
                test_id: "workspace-root",
                accessibility_label: "Aether Studio Workspace",
                "layout-engine": "{layout_engine_str}",
                "viewport-mode": "{viewport_mode_str}",
                style: "width: 100%; height: 100%; flex-direction: {root_flex_dir}; background-color: {CANVAS_BG};",

                // 1A. MOBILE PORTRAIT COMPACT TOP BAR + COLLAPSIBLE DRAWERS (< 620pt)
                if is_mobile {
                    view {
                        test_id: "mobile-top-bar",
                        style: "width: 100%; height: 42px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {PANEL_BG}; border-width: {panel_bw}px; border-color: {panel_bc};",
                        view {
                            style: "flex-direction: row; align-items: center; gap: 8px;",
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                test_id: Some("mobile-toggle-nav".into()),
                                style: Some(mnav_btn_style.clone()),
                                on_press: move |_| mutate_state(&s_toggle_nav_mobile, version, |st| st.toggle_mobile_nav()),
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
                                style: Some(mtask_btn_style.clone()),
                                on_press: move |_| mutate_state(&s_new_task_mobile, version, |st| st.create_new_task()),
                                "+ Task"
                            }
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                test_id: Some("btn-toggle-studio".into()),
                                style: Some(mstudio_btn_style.clone()),
                                on_press: move |_| mutate_state(&s_toggle_studio_mobile, version, |st| st.toggle_studio_panel()),
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
                                    let s_nav = Arc::clone(&store);
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

                    if current.mobile_studio_open {
                        {render_studio_panel(&store, version, &current, &sync_telemetry, snap_idx, hist_len, can_undo, can_redo, m, true)}
                    }
                }

                // 1B. DESKTOP LEFT SIDEBAR (172px fixed, >= 620pt)
                if !is_mobile {
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
                                    let s_nav = Arc::clone(&store);
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

                // 2. CENTER MAIN WORKSPACE
                view {
                    test_id: "center-workspace",
                    style: "{center_style}",

                    Card {
                        variant: CardVariant::Outlined,
                        padding: Some(6.0),
                        gap: Some(6.0),
                        style: Some(header_card_style),
                        text {
                            style: "color: {TEXT_PRIMARY}; font-size: 12px;",
                            "{active_view.title()}"
                        }
                        view {
                            style: "flex-direction: row; align-items: center; gap: 5px; flex-shrink: 0;",
                            Button {
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                test_id: Some("btn-command-palette".into()),
                                accessibility_label: Some("Open Cmd+K Command Palette".into()),
                                style: Some(cmdk_btn_style),
                                on_press: move |_| mutate_state(&s_toggle_palette, version, |st| st.toggle_palette()),
                                "Cmd+K"
                            }
                            Badge {
                                label: status_pill_label,
                                variant: BadgeVariant::Neutral,
                                style: Some(status_pill_style),
                            }
                        }
                    }

                    if palette.open {
                        view {
                            style: "width: 100%; flex: 1; flex-direction: column;",
                            {render_command_palette(&store, version, &palette, &palette_matches, accent_hex, accent_tint, xray, m)}
                        }
                    } else {
                        match active_view {
                            ActiveView::Board => {
                                render_board_view(&store, version, &current, &filtered_tasks, total_tasks, active_tasks, done_tasks, accent_hex, accent_tint, xray, m)
                            }
                            ActiveView::ChannelEngineering | ActiveView::ChannelDesign => {
                                let ch = active_channel.unwrap_or(ChannelKind::Engineering);
                                render_channel_view(&store, version, ch, channel_messages, channel_draft, accent_hex, accent_tint, xray, m)
                            }
                            ActiveView::Docs => {
                                render_docs_view(&store, version, &current, accent_hex, accent_tint, xray, m, is_mobile)
                            }
                            ActiveView::Workbench => {
                                render_workbench_view(&store, version, &current, &sync_telemetry, accent_hex, accent_tint, xray, m, is_mobile)
                            }
                            ActiveView::Stream100k => {
                                rsx! {
                                    Stream100kPanel {
                                        store: SharedStore(Arc::clone(&store)),
                                        accent,
                                        density: studio.density,
                                        xray,
                                        pill_radius: m.pill_radius,
                                        button_radius: m.button_radius,
                                        is_mobile,
                                        parent_rev: rev,
                                    }
                                }
                            }
                            ActiveView::Islands => {
                                render_islands_view(&store, version, &current.islands, accent_hex, accent_tint, xray, m)
                            }
                            ActiveView::Metrics => {
                                render_metrics_view(&store, version, &current, frame_num, snap_idx, hist_len, active_tasks, total_msgs, accent_hex, accent_tint, xray, m)
                            }
                        }
                    }
                }

                // 3. RIGHT DOCKED NVE STUDIO META-WORKBENCH (desktop >= 620pt when open)
                if !is_mobile && studio.panel_open {
                    {render_studio_panel(&store, version, &current, &sync_telemetry, snap_idx, hist_len, can_undo, can_redo, m, false)}
                }
            }
        }
    }
}
