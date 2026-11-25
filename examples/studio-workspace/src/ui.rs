//! Dioxus 0.7 `rsx!` UI for **Aether Workspace** + docked **NVE Studio** meta-workbench.
//!
//! Exercises every NVE primitive (`View`, `Text`, `Pressable`, `ScrollView`, `TextInput`,
//! `Image`, `SkiaCanvas`) using the typed CSS/attribute subset so the emitted single `nve-abi`
//! binary `CommandBuffer` lays out and renders identically across macOS AppKit, Qt 6, Skia
//! Desktop, Web WASM, and Headless Taffy/Yoga backends.
//!
//! Automatically adapts between the desktop 3-column workspace (`>= 620pt`) and a responsive
//! single-column mobile portrait shell (`< 620pt`, e.g. iPhone 17 393pt) with collapsible top
//! navigation and NVE Studio drawers so the center board is never crushed.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;

use crate::assets::{avatar_for_author, build_metrics_sparkline};
use crate::state::{ActiveView, BoardFilter, ChannelKind, WorkspaceState};
use crate::theme::{
    card_border, control_border, panel_border, row_border, LayoutMetrics, BORDER_DEFAULT,
    CANVAS_BG, CARD_ALT_BG, CARD_BG, CONTROL_BG, PANEL_BG, TEXT_MUTED, TEXT_PRIMARY,
    TEXT_SECONDARY, XRAY_ROW,
};
use crate::ui_islands::render_islands_view;
use crate::ui_palette::{filter_palette_items, render_command_palette};
use crate::ui_stream::{SharedStore, Stream100kPanel};
use crate::ui_studio::render_studio_panel;

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
    let islands_count = current.islands.cards.len();
    let stress_rows = current.stress_rows;

    let nav_items: &[(ActiveView, &str, &str, String)] = &[
        (
            ActiveView::Board,
            "nav-board",
            "Board",
            total_tasks.to_string(),
        ),
        (
            ActiveView::ChannelEngineering,
            "nav-engineering",
            "# engineering",
            eng_count.to_string(),
        ),
        (
            ActiveView::ChannelDesign,
            "nav-design",
            "# design-sync",
            design_count.to_string(),
        ),
        (
            ActiveView::Stream100k,
            "nav-stream",
            "100k Stream",
            "100k".into(),
        ),
        (
            ActiveView::Islands,
            "nav-islands",
            "Islands",
            islands_count.to_string(),
        ),
        (
            ActiveView::Metrics,
            "nav-metrics",
            "Metrics",
            stress_rows.to_string(),
        ),
    ];

    let sidebar_style = format!(
        "width: 172px; height: 100%; flex-shrink: 0; flex-direction: column; padding: 12px; gap: 8px; background-color: {PANEL_BG}; border-width: {panel_bw}px; border-color: {panel_bc};"
    );

    let brand_box_style = format!(
        "width: 100%; flex-shrink: 0; flex-direction: column; gap: 2px; padding: 8px; background-color: {CARD_ALT_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );

    let new_task_style = format!(
        "width: 100%; height: 30px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {accent_hex}; border-radius: {}px; border-width: {}px; border-color: {};",
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
        "width: 100%; height: 30px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {studio_btn_bg}; border-radius: {}px; border-width: {studio_btn_bw}px; border-color: {studio_btn_bc};",
        m.button_radius
    );
    let studio_badge_bg = if studio_open_active {
        accent_hex
    } else {
        "#334155"
    };
    let studio_badge_fg = if studio_open_active {
        accent.on_accent_fg()
    } else {
        TEXT_SECONDARY
    };
    let studio_badge_text = if studio_open_active { "ON" } else { "OFF" };

    let center_pad = if is_mobile { m.main_pad.min(10) } else { m.main_pad };
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

    let header_bar_style = format!(
        "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 8px; padding-vertical: 6px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );

    let status_pill_style = format!(
        "height: 22px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 6px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        m.pill_radius
    );
    let compact_header = current.viewport_width < 760;
    let status_pill_label = if compact_header {
        format!("F#{frame_num} {snap_idx}/{hist_len}")
    } else {
        format!("Frame #{frame_num} | {snap_idx}/{hist_len}")
    };
    let (cmdk_bw, cmdk_bc) = control_border(xray, palette.open, accent_hex);
    let cmdk_bg = if palette.open { accent_tint } else { CONTROL_BG };
    let cmdk_fg = if palette.open {
        accent_hex
    } else {
        TEXT_SECONDARY
    };

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

    let sparkline_attr = if active_view == ActiveView::Metrics {
        build_metrics_sparkline(&current.telemetry_samples_us, hist_len, accent_hex)
            .to_attr_string()
    } else {
        String::new()
    };

    let root_flex_dir = if is_mobile { "column" } else { "row" };

    let s_new_task = Arc::clone(&store);
    let s_new_task_mobile = Arc::clone(&store);
    let s_toggle_studio = Arc::clone(&store);
    let s_toggle_studio_mobile = Arc::clone(&store);
    let s_toggle_nav_mobile = Arc::clone(&store);
    let s_toggle_palette = Arc::clone(&store);
    let s_board_search = Arc::clone(&store);
    let s_board_search_clear = Arc::clone(&store);
    let s_board_draft = Arc::clone(&store);
    let s_board_draft_submit = Arc::clone(&store);
    let s_board_draft_btn = Arc::clone(&store);
    let s_stress8 = Arc::clone(&store);
    let s_stress24 = Arc::clone(&store);
    let s_stress_rst = Arc::clone(&store);

    let (mnav_bw, mnav_bc) = control_border(xray, current.mobile_nav_open, accent_hex);
    let mnav_bg = if current.mobile_nav_open {
        accent_tint
    } else {
        CONTROL_BG
    };
    let mnav_lbl = if current.mobile_nav_open {
        "Close"
    } else {
        "Menu"
    };

    rsx! {
        view {
            test_id: "workspace-root",
            accessibility_label: "Aether Studio Workspace",
            "layout-engine": "{layout_engine_str}",
            "viewport-mode": "{viewport_mode_str}",
            style: "width: 100%; height: 100%; flex-direction: {root_flex_dir}; background-color: {CANVAS_BG};",

            // =========================================================================
            // 1A. MOBILE PORTRAIT COMPACT TOP BAR + COLLAPSIBLE DRAWERS (< 620pt)
            // =========================================================================
            if is_mobile {
                view {
                    test_id: "mobile-top-bar",
                    style: "width: 100%; height: 42px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {PANEL_BG}; border-width: {panel_bw}px; border-color: {panel_bc};",
                    view {
                        style: "flex-direction: row; align-items: center; gap: 8px;",
                        pressable {
                            test_id: "mobile-toggle-nav",
                            style: "width: 52px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {mnav_bg}; border-radius: {m.pill_radius}px; border-width: {mnav_bw}px; border-color: {mnav_bc};",
                            onpress: move |_| mutate_state(&s_toggle_nav_mobile, version, |st| st.toggle_mobile_nav()),
                            text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "{mnav_lbl}" }
                        }
                        text {
                            style: "color: {accent_hex}; font-size: 13px;",
                            "AETHER"
                        }
                    }
                    view {
                        style: "flex-direction: row; align-items: center; gap: 6px;",
                        pressable {
                            test_id: "btn-new-task",
                            style: "width: 58px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {accent_hex}; border-radius: {m.pill_radius}px;",
                            onpress: move |_| mutate_state(&s_new_task_mobile, version, |st| st.create_new_task()),
                            text { style: "color: #0b1120; font-size: 10px;", "+ Task" }
                        }
                        pressable {
                            test_id: "btn-toggle-studio",
                            style: "width: 66px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {studio_btn_bg}; border-radius: {m.pill_radius}px; border-width: {studio_btn_bw}px; border-color: {studio_btn_bc};",
                            onpress: move |_| mutate_state(&s_toggle_studio_mobile, version, |st| st.toggle_studio_panel()),
                            text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "Studio {studio_badge_text}" }
                        }
                    }
                }

                if current.mobile_nav_open {
                    view {
                        test_id: "mobile-nav-drawer",
                        style: "width: 100%; flex-shrink: 0; flex-direction: column; padding: 8px; gap: 5px; background-color: {PANEL_BG}; border-width: {card_bw}px; border-color: {card_bc};",
                        for (view_kind, tid, label, count) in nav_items.iter() {
                            {
                                let vk = *view_kind;
                                let is_active = active_view == vk;
                                let (bw, bc) = control_border(xray, is_active, accent_hex);
                                let bg = if is_active { accent_tint } else { CONTROL_BG };
                                let fg = if is_active { TEXT_PRIMARY } else { TEXT_SECONDARY };
                                let count_bg = if is_active { accent_hex } else { "#334155" };
                                let count_fg = if is_active { accent.on_accent_fg() } else { TEXT_SECONDARY };
                                let s_nav = Arc::clone(&store);
                                rsx! {
                                    pressable {
                                        key: "m-{tid}",
                                        test_id: "{tid}",
                                        style: "width: 100%; height: 28px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {bg}; border-radius: {m.button_radius}px; border-width: {bw}px; border-color: {bc};",
                                        onpress: move |_| mutate_state(&s_nav, version, |st| st.select_view(vk)),
                                        text { style: "color: {fg}; font-size: 11px;", "{label}" }
                                        view {
                                            style: "min-width: 24px; height: 18px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 5px; background-color: {count_bg}; border-radius: {m.pill_radius}px;",
                                            text { style: "color: {count_fg}; font-size: 10px;", "{count}" }
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

            // =========================================================================
            // 1B. DESKTOP LEFT SIDEBAR (172px fixed, >= 620pt)
            // =========================================================================
            if !is_mobile {
                view {
                    test_id: "desktop-sidebar",
                    style: "{sidebar_style}",

                    // Brand Header
                    view {
                        style: "{brand_box_style}",
                        text {
                            style: "color: {accent_hex}; font-size: 15px;",
                            "AETHER"
                        }
                        text {
                            style: "color: {TEXT_MUTED}; font-size: 10px;",
                            "LOCAL-FIRST | NVE"
                        }
                    }

                    // Navigation Tabs
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 5px; border-width: {row_bw}px; border-color: {row_bc};",
                        for (view_kind, tid, label, count) in nav_items.iter() {
                            {
                                let vk = *view_kind;
                                let is_active = active_view == vk;
                                let (bw, bc) = control_border(xray, is_active, accent_hex);
                                let bg = if is_active { accent_tint } else { CONTROL_BG };
                                let fg = if is_active { TEXT_PRIMARY } else { TEXT_SECONDARY };
                                let count_bg = if is_active { accent_hex } else { "#334155" };
                                let count_fg = if is_active { accent.on_accent_fg() } else { TEXT_SECONDARY };
                                let tab_style = format!(
                                    "width: 100%; height: 28px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 9px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                                    m.button_radius
                                );
                                let badge_style = format!(
                                    "min-width: 20px; height: 18px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 5px; background-color: {count_bg}; border-radius: {}px;",
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
                                        view {
                                            style: "{badge_style}",
                                            text {
                                                style: "color: {count_fg}; font-size: 10px;",
                                                "{count}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Flexible Spacer
                    view {
                        style: "width: 100%; flex: 1;",
                    }

                    // Bottom Actions
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 6px; border-width: {row_bw}px; border-color: {row_bc};",
                        pressable {
                            test_id: "btn-new-task",
                            style: "{new_task_style}",
                            onpress: move |_| mutate_state(&s_new_task, version, |st| st.create_new_task()),
                            text {
                                style: "color: #0b1120; font-size: 12px;",
                                "+ New Task"
                            }
                        }
                        pressable {
                            test_id: "btn-toggle-studio",
                            style: "{studio_toggle_style}",
                            onpress: move |_| mutate_state(&s_toggle_studio, version, |st| st.toggle_studio_panel()),
                            text {
                                style: "color: {TEXT_PRIMARY}; font-size: 11px;",
                                "NVE Studio"
                            }
                            view {
                                style: "height: 18px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 6px; background-color: {studio_badge_bg}; border-radius: {m.pill_radius}px;",
                                text {
                                    style: "color: {studio_badge_fg}; font-size: 10px;",
                                    "{studio_badge_text}"
                                }
                            }
                        }
                    }
                }
            }

            // =========================================================================
            // 2. CENTER MAIN WORKSPACE (flex: 1 — full width on mobile portrait)
            // =========================================================================
            view {
                test_id: "center-workspace",
                style: "{center_style}",

                // Top Header Bar with Cmd+K Command Palette trigger + Status Pill
                view {
                    style: "{header_bar_style}",
                    text {
                        style: "color: {TEXT_PRIMARY}; font-size: 12px;",
                        "{active_view.title()}"
                    }
                    view {
                        style: "flex-direction: row; align-items: center; gap: 5px; flex-shrink: 0;",
                        pressable {
                            test_id: "btn-command-palette",
                            accessibility_label: "Open Cmd+K Command Palette",
                            accessibility_role: "button",
                            style: "width: 46px; height: 22px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {cmdk_bg}; border-radius: {m.pill_radius}px; border-width: {cmdk_bw}px; border-color: {cmdk_bc};",
                            onpress: move |_| mutate_state(&s_toggle_palette, version, |st| st.toggle_palette()),
                            text { style: "color: {cmdk_fg}; font-size: 9px;", "Cmd+K" }
                        }
                        view {
                            style: "{status_pill_style}",
                            text {
                                style: "color: {accent_hex}; font-size: 10px;",
                                "{status_pill_label}"
                            }
                        }
                    }
                }

                // ---- CMD+K COMMAND PALETTE OVERLAY ----
                if palette.open {
                    {render_command_palette(&store, version, &palette, &palette_matches, accent_hex, accent_tint, xray, m)}
                }

                // ---- BOARD VIEW ----
                if active_view == ActiveView::Board {
                    // Filter Pills Row
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px; border-width: {row_bw}px; border-color: {row_bc};",
                        for &(flt, tid, label, cnt, pill_w) in [
                            (BoardFilter::All, "filter-all", "All", total_tasks, 64u32),
                            (BoardFilter::Active, "filter-active", "Active", active_tasks, 78u32),
                            (BoardFilter::Done, "filter-done", "Done", done_tasks, 68u32),
                        ].iter() {
                            {
                                let is_sel = current.board_filter == flt;
                                let (bw, bc) = control_border(xray, is_sel, accent_hex);
                                let bg = if is_sel { accent_tint } else { PANEL_BG };
                                let fg = if is_sel { accent_hex } else { TEXT_SECONDARY };
                                let flt_style = format!(
                                    "width: {pill_w}px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                                    m.pill_radius
                                );
                                let text_val = format!("{label} ({cnt})");
                                let s_flt = Arc::clone(&store);
                                rsx! {
                                    pressable {
                                        key: "{tid}",
                                        test_id: "{tid}",
                                        style: "{flt_style}",
                                        onpress: move |_| mutate_state(&s_flt, version, |st| st.set_board_filter(flt)),
                                        text {
                                            style: "color: {fg}; font-size: 10px;",
                                            "{text_val}"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Live Board Search & Custom Task Create Bar (real TextInput primitives)
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 5px; border-width: {row_bw}px; border-color: {row_bc};",
                        view {
                            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 5px; align-items: center;",
                            textinput {
                                test_id: "board-search-input",
                                placeholder: "Filter issues...",
                                value: "{current.board_search_query}",
                                multiline: false,
                                accessibility_label: "Filter board issues",
                                accessibility_role: "searchbox",
                                style: "flex: 1; height: 26px; padding-horizontal: 8px; background-color: {PANEL_BG}; color: {TEXT_PRIMARY}; font-size: 11px; border-radius: {m.button_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                                onchange: move |ev| {
                                    let q = ev.data().value.clone();
                                    mutate_state(&s_board_search, version, |st| st.set_board_search_query(q));
                                },
                            }
                            if !current.board_search_query.is_empty() {
                                pressable {
                                    test_id: "board-search-clear",
                                    style: "width: 40px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                                    onpress: move |_| mutate_state(&s_board_search_clear, version, |st| st.set_board_search_query("")),
                                    text { style: "color: {TEXT_SECONDARY}; font-size: 9px;", "Clear" }
                                }
                            }
                        }
                        view {
                            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 5px; align-items: center;",
                            textinput {
                                test_id: "board-new-task-input",
                                placeholder: "New custom issue title...",
                                value: "{current.new_task_draft}",
                                multiline: false,
                                accessibility_label: "New issue title",
                                style: "flex: 1; height: 26px; padding-horizontal: 8px; background-color: {PANEL_BG}; color: {TEXT_PRIMARY}; font-size: 11px; border-radius: {m.button_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                                onchange: move |ev| {
                                    let txt = ev.data().value.clone();
                                    mutate_state(&s_board_draft, version, |st| st.set_new_task_draft(txt));
                                },
                                onsubmit: move |_| {
                                    mutate_state(&s_board_draft_submit, version, |st| {
                                        let _ = st.submit_new_task_draft();
                                    });
                                },
                            }
                            pressable {
                                test_id: "board-new-task-submit",
                                style: "width: 50px; height: 26px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {accent_tint}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {accent_hex};",
                                onpress: move |_| mutate_state(&s_board_draft_btn, version, |st| {
                                    let _ = st.submit_new_task_draft();
                                }),
                                text { style: "color: {accent_hex}; font-size: 10px;", "+ Add" }
                            }
                        }
                    }

                    // Scrollable Task Card List (with real PNG Image avatars)
                    scrollview {
                        style: "width: 100%; flex: 1; flex-direction: column; gap: {m.list_gap}px; border-width: {row_bw}px; border-color: {row_bc};",
                        for task in filtered_tasks.iter() {
                            {
                                let tid = task.id;
                                let status_tid = format!("task-status-{tid}");
                                let avatar_tid = format!("task-avatar-{tid}");
                                let avatar_src = avatar_for_author(&task.assignee);
                                let card_style = format!(
                                    "width: 100%; flex-shrink: 0; flex-direction: column; padding: {}px; gap: {}px; background-color: {CARD_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
                                    m.card_pad, m.card_gap, m.card_radius
                                );
                                let p_bg = task.priority.badge_bg();
                                let p_fg = task.priority.badge_fg();
                                let p_bc = task.priority.badge_border();
                                let p_lbl = task.priority.label();
                                let st_bg = task.status.pill_bg();
                                let st_fg = task.status.pill_fg();
                                let st_bc = if xray { XRAY_ROW } else { task.status.pill_border() };
                                let st_lbl = task.status.label();
                                let meta_line = format!("{} | {}", task.assignee, task.tag);
                                let s_cyc = Arc::clone(&store);
                                rsx! {
                                    view {
                                        key: "task-{tid}",
                                        style: "{card_style}",
                                        // Top row: Key + Priority pill | Clickable Status pill
                                        view {
                                            style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; border-width: {row_bw}px; border-color: {row_bc};",
                                            view {
                                                style: "flex-direction: row; align-items: center; gap: 6px;",
                                                view {
                                                    style: "width: 28px; height: 20px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {p_bg}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {p_bc};",
                                                    text {
                                                        style: "color: {p_fg}; font-size: 10px;",
                                                        "{p_lbl}"
                                                    }
                                                }
                                                text {
                                                    style: "color: {TEXT_MUTED}; font-size: 11px;",
                                                    "{task.key}"
                                                }
                                            }
                                            pressable {
                                                test_id: "{status_tid}",
                                                style: "width: 88px; height: 24px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {st_bg}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {st_bc};",
                                                onpress: move |_| mutate_state(&s_cyc, version, |st| st.cycle_task_status(tid)),
                                                text {
                                                    style: "color: {st_fg}; font-size: 11px;",
                                                    "{st_lbl}"
                                                }
                                            }
                                        }
                                        // Task Title
                                        text {
                                            style: "color: {TEXT_PRIMARY}; font-size: 12px;",
                                            "{task.title}"
                                        }
                                        // Assignee Avatar Image & Tag row
                                        view {
                                            style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 6px;",
                                            image {
                                                test_id: "{avatar_tid}",
                                                src: "{avatar_src}",
                                                alt: "{task.assignee}",
                                                resize_mode: "cover",
                                                accessibility_label: "Assignee avatar {task.assignee}",
                                                style: "width: 18px; height: 18px; flex-shrink: 0; border-radius: 9px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                                            }
                                            text {
                                                style: "color: {TEXT_MUTED}; font-size: 11px;",
                                                "{meta_line}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // ---- CHANNEL VIEWS (# engineering & # design-sync) ----
                if let Some(ch) = active_channel {
                    scrollview {
                        style: "width: 100%; flex: 1; flex-direction: column; gap: {m.list_gap}px; border-width: {row_bw}px; border-color: {row_bc};",
                        for msg in channel_messages.iter() {
                            {
                                let mid = msg.id;
                                let react_tid = format!("msg-react-{mid}");
                                let avatar_tid = format!("msg-avatar-{mid}");
                                let avatar_src = avatar_for_author(&msg.author);
                                let card_style = format!(
                                    "width: 100%; flex-shrink: 0; flex-direction: column; padding: {}px; gap: {}px; background-color: {CARD_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
                                    m.card_pad, m.card_gap, m.card_radius
                                );
                                let react_lbl = format!("+1  {}", msg.reactions);
                                let react_bc = if xray { XRAY_ROW } else { accent_hex };
                                let s_react = Arc::clone(&store);
                                rsx! {
                                    view {
                                        key: "msg-{mid}",
                                        style: "{card_style}",
                                        view {
                                            style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; border-width: {row_bw}px; border-color: {row_bc};",
                                            view {
                                                style: "flex-direction: row; align-items: center; gap: 6px;",
                                                image {
                                                    test_id: "{avatar_tid}",
                                                    src: "{avatar_src}",
                                                    alt: "{msg.author}",
                                                    resize_mode: "cover",
                                                    accessibility_label: "Author avatar {msg.author}",
                                                    style: "width: 18px; height: 18px; flex-shrink: 0; border-radius: 9px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                                                }
                                                view {
                                                    style: "height: 20px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 7px; background-color: {msg.badge_bg}; border-radius: {m.pill_radius}px;",
                                                    text {
                                                        style: "color: {msg.badge_fg}; font-size: 10px;",
                                                        "{msg.author}"
                                                    }
                                                }
                                                text {
                                                    style: "color: {TEXT_MUTED}; font-size: 10px;",
                                                    "{msg.timestamp}"
                                                }
                                            }
                                            pressable {
                                                test_id: "{react_tid}",
                                                style: "width: 58px; height: 24px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {accent_tint}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {react_bc};",
                                                onpress: move |_| mutate_state(&s_react, version, |st| st.react_message(ch, mid)),
                                                text {
                                                    style: "color: {accent_hex}; font-size: 11px;",
                                                    "{react_lbl}"
                                                }
                                            }
                                        }
                                        text {
                                            style: "color: {TEXT_PRIMARY}; font-size: 11px;",
                                            "{msg.body}"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Live TextInput Chat Composer + Quick-Reply Chips
                    {
                        let composer_placeholder = format!("Message {}...", ch.tag());
                        let s_chat_input = Arc::clone(&store);
                        let s_chat_submit = Arc::clone(&store);
                        let s_chat_send = Arc::clone(&store);
                        rsx! {
                            view {
                                style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 6px; padding: 9px; background-color: {PANEL_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                                view {
                                    style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px; align-items: center;",
                                    textinput {
                                        test_id: "chat-composer-input",
                                        placeholder: "{composer_placeholder}",
                                        value: "{channel_draft}",
                                        multiline: false,
                                        accessibility_label: "Channel message input",
                                        style: "flex: 1; height: 28px; padding-horizontal: 8px; background-color: {CARD_ALT_BG}; color: {TEXT_PRIMARY}; font-size: 11px; border-radius: {m.button_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                                        onchange: move |ev| {
                                            let txt = ev.data().value.clone();
                                            mutate_state(&s_chat_input, version, |st| st.set_channel_draft(ch, txt));
                                        },
                                        onsubmit: move |_| {
                                            mutate_state(&s_chat_submit, version, |st| {
                                                let _ = st.submit_channel_draft(ch);
                                            });
                                        },
                                    }
                                    pressable {
                                        test_id: "chat-composer-send",
                                        style: "width: 48px; height: 28px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {accent_hex}; border-radius: {m.button_radius}px;",
                                        onpress: move |_| mutate_state(&s_chat_send, version, |st| {
                                            let _ = st.submit_channel_draft(ch);
                                        }),
                                        text { style: "color: #0b1120; font-size: 10px;", "Send" }
                                    }
                                }
                                text {
                                    style: "color: {TEXT_MUTED}; font-size: 10px;",
                                    "QUICK REPLY"
                                }
                                view {
                                    style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px;",
                                    for &(tid, chip_label, reply_body) in [
                                        ("reply-ship", "Ship It", "Ship It - all checks green!"),
                                        ("reply-bench", "Run Benches", "Run Benches - checking p99."),
                                        ("reply-lgtm", "LGTM", "LGTM - clean single-ABI PR."),
                                    ].iter() {
                                        {
                                            let (bw, bc) = control_border(xray, false, accent_hex);
                                            let chip_style = format!(
                                                "flex: 1; height: 26px; justify-content: center; align-items: center; background-color: {CARD_BG}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                                                m.button_radius
                                            );
                                            let s_rep = Arc::clone(&store);
                                            rsx! {
                                                pressable {
                                                    key: "{tid}",
                                                    test_id: "{tid}",
                                                    style: "{chip_style}",
                                                    onpress: move |_| mutate_state(&s_rep, version, |st| st.send_quick_reply(ch, reply_body)),
                                                    text {
                                                        style: "color: {TEXT_PRIMARY}; font-size: 10px;",
                                                        "{chip_label}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // ---- 100K VIRTUALIZED STREAM VIEW ----
                if active_view == ActiveView::Stream100k {
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

                // ---- MULTI-HOST <ISLAND> SANDBOX VIEW ----
                if active_view == ActiveView::Islands {
                    {render_islands_view(&store, version, &current.islands, accent_hex, accent_tint, xray, m)}
                }

                // ---- METRICS VIEW ----
                if active_view == ActiveView::Metrics {
                    // 2x2 KPI Grid
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 8px;",
                        view {
                            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 8px;",
                            view {
                                style: "flex: 1; flex-direction: column; padding: 10px; gap: 4px; background-color: {CARD_BG}; border-radius: {m.card_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                                text { style: "color: {TEXT_MUTED}; font-size: 10px;", "FRAMES" }
                                text { style: "color: {accent_hex}; font-size: 18px;", "{frame_num}" }
                            }
                            view {
                                style: "flex: 1; flex-direction: column; padding: 10px; gap: 4px; background-color: {CARD_BG}; border-radius: {m.card_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                                text { style: "color: {TEXT_MUTED}; font-size: 10px;", "SNAPSHOTS" }
                                text { style: "color: #a78bfa; font-size: 18px;", "{snap_idx}/{hist_len}" }
                            }
                        }
                        view {
                            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 8px;",
                            view {
                                style: "flex: 1; flex-direction: column; padding: 10px; gap: 4px; background-color: {CARD_BG}; border-radius: {m.card_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                                text { style: "color: {TEXT_MUTED}; font-size: 10px;", "ACTIVE TASKS" }
                                text { style: "color: #22c55e; font-size: 18px;", "{active_tasks}" }
                            }
                            view {
                                style: "flex: 1; flex-direction: column; padding: 10px; gap: 4px; background-color: {CARD_BG}; border-radius: {m.card_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                                text { style: "color: {TEXT_MUTED}; font-size: 10px;", "MESSAGES" }
                                text { style: "color: #f59e0b; font-size: 18px;", "{total_msgs}" }
                            }
                        }
                    }

                    // Live SkiaCanvas Telemetry Sparkline emitting NVE_DRAW1 binary draw-list IR
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 4px; padding: 8px; background-color: {PANEL_BG}; border-radius: {m.card_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                        view {
                            style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                            text { style: "color: {TEXT_MUTED}; font-size: 10px;", "SKIA SPARKLINE (NVE_DRAW1)" }
                            text { style: "color: {accent_hex}; font-size: 10px;", "{current.telemetry_samples_us.len()} pts" }
                        }
                        canvas {
                            test_id: "metrics-sparkline-canvas",
                            draw_list: "{sparkline_attr}",
                            accessibility_label: "Live telemetry frame timing sparkline",
                            accessibility_role: "image",
                            style: "width: 100%; height: 72px; flex-shrink: 0; background-color: {CARD_ALT_BG}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                        }
                    }

                    // Stress Controls Row
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 6px; border-width: {row_bw}px; border-color: {row_bc};",
                        pressable {
                            test_id: "stress-add-8",
                            style: "flex: 1; height: 28px; justify-content: center; align-items: center; background-color: {accent_tint}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {accent_hex};",
                            onpress: move |_| mutate_state(&s_stress8, version, |st| st.add_stress_rows(8)),
                            text { style: "color: {accent_hex}; font-size: 11px;", "+8 Rows" }
                        }
                        pressable {
                            test_id: "stress-add-24",
                            style: "flex: 1; height: 28px; justify-content: center; align-items: center; background-color: {accent_tint}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {accent_hex};",
                            onpress: move |_| mutate_state(&s_stress24, version, |st| st.add_stress_rows(24)),
                            text { style: "color: {accent_hex}; font-size: 11px;", "+24 Rows" }
                        }
                        pressable {
                            test_id: "stress-reset",
                            style: "flex: 1; height: 28px; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.button_radius}px; border-width: 1px; border-color: #334155;",
                            onpress: move |_| mutate_state(&s_stress_rst, version, |st| st.reset_stress_rows()),
                            text { style: "color: {TEXT_SECONDARY}; font-size: 11px;", "Reset" }
                        }
                    }

                    // Live Stress Metric Rows
                    scrollview {
                        style: "width: 100%; flex: 1; flex-direction: column; gap: 5px; border-width: {row_bw}px; border-color: {row_bc};",
                        for row_idx in 1..=stress_rows {
                            {
                                let us = 180 + ((row_idx * 17) % 140);
                                let row_bg = if row_idx % 2 == 0 { CARD_BG } else { CARD_ALT_BG };
                                let row_lbl = format!("Node Batch #{row_idx:02}");
                                let timing_lbl = format!("{us}us | OK");
                                let (rbw, rbc) = if xray { (1, XRAY_ROW) } else { (1, "#1e293b") };
                                rsx! {
                                    view {
                                        key: "metric-row-{row_idx}",
                                        style: "width: 100%; height: 28px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {row_bg}; border-radius: {m.pill_radius}px; border-width: {rbw}px; border-color: {rbc};",
                                        text { style: "color: {TEXT_SECONDARY}; font-size: 11px;", "{row_lbl}" }
                                        text { style: "color: #22c55e; font-size: 11px;", "{timing_lbl}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // =========================================================================
            // 3. RIGHT DOCKED NVE STUDIO META-WORKBENCH (desktop >= 620pt when open)
            // =========================================================================
            if !is_mobile && studio.panel_open {
                {render_studio_panel(&store, version, &current, &sync_telemetry, snap_idx, hist_len, can_undo, can_redo, m, false)}
            }
        }
    }
}
