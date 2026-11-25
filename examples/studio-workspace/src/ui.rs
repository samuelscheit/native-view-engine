//! Dioxus 0.7 `rsx!` UI for **Aether Workspace** + docked **NVE Studio** meta-workbench.
//!
//! Uses exclusively the typed CSS subset parsed by `nve-components` so the emitted single
//! `nve-abi` binary `CommandBuffer` lays out and renders identically across macOS AppKit,
//! Qt 6, Skia Desktop, Web WASM, and Headless Taffy/Yoga backends.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;

use crate::state::{
    AccentTheme, ActiveView, BoardFilter, ChannelKind, CornerRadiusMode, DensityMode,
    WorkspaceState,
};
use crate::theme::{
    card_border, control_border, panel_border, row_border, LayoutMetrics, CANVAS_BG, CARD_ALT_BG,
    CARD_BG, CONTROL_BG, PANEL_BG, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY, XRAY_ROW,
};

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

fn mutate_state(
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
    let _rev = version();

    let snapshot = store
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|e| e.into_inner().clone());

    let studio = &snapshot.current.studio;
    let accent = studio.accent;
    let accent_hex = accent.hex();
    let accent_tint = accent.tint_bg();
    let xray = studio.xray;
    let m = LayoutMetrics::from_studio(studio);

    let (panel_bw, panel_bc) = panel_border(xray);
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);

    let active_view = snapshot.current.active_view;
    let total_tasks = snapshot.current.tasks.len();
    let active_tasks = snapshot.active_task_count();
    let done_tasks = snapshot.done_task_count();
    let eng_count = snapshot.current.eng_messages.len();
    let design_count = snapshot.current.design_messages.len();
    let total_msgs = snapshot.total_message_count();
    let stress_rows = snapshot.current.stress_rows;
    let frame_num = snapshot.frame_count;
    let snap_idx = snapshot.snapshot_index_1based();
    let hist_len = snapshot.history_len();
    let can_undo = snapshot.can_undo();
    let can_redo = snapshot.can_redo();
    let action_label = snapshot.current.action_label.clone();

    let nav_items: &[(ActiveView, &str, &str, usize)] = &[
        (ActiveView::Board, "nav-board", "Board", total_tasks),
        (
            ActiveView::ChannelEngineering,
            "nav-engineering",
            "# engineering",
            eng_count,
        ),
        (
            ActiveView::ChannelDesign,
            "nav-design",
            "# design-sync",
            design_count,
        ),
        (ActiveView::Metrics, "nav-metrics", "Metrics", stress_rows),
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

    let (studio_btn_bw, studio_btn_bc) = control_border(xray, studio.panel_open, accent_hex);
    let studio_btn_bg = if studio.panel_open {
        accent_tint
    } else {
        CONTROL_BG
    };
    let studio_toggle_style = format!(
        "width: 100%; height: 30px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; background-color: {studio_btn_bg}; border-radius: {}px; border-width: {studio_btn_bw}px; border-color: {studio_btn_bc};",
        m.button_radius
    );
    let studio_badge_bg = if studio.panel_open {
        accent_hex
    } else {
        "#334155"
    };
    let studio_badge_fg = if studio.panel_open {
        accent.on_accent_fg()
    } else {
        TEXT_SECONDARY
    };
    let studio_badge_text = if studio.panel_open { "ON" } else { "OFF" };

    let center_style = format!(
        "flex: 1; height: 100%; flex-direction: column; padding: {}px; gap: {}px; background-color: {CANVAS_BG}; border-width: {panel_bw}px; border-color: {panel_bc};",
        m.main_pad, m.main_gap
    );

    let header_bar_style = format!(
        "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 10px; padding-vertical: 7px; background-color: {PANEL_BG}; border-radius: {}px; border-width: {card_bw}px; border-color: {card_bc};",
        m.button_radius
    );

    let status_pill_style = format!(
        "height: 22px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 8px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        m.pill_radius
    );
    let status_pill_label = format!("Frame #{frame_num} | {snap_idx}/{hist_len}");

    let studio_panel_style = format!(
        "width: 210px; height: 100%; flex-shrink: 0; flex-direction: column; padding: 10px; gap: 7px; background-color: {PANEL_BG}; border-width: {panel_bw}px; border-color: {panel_bc};"
    );

    let filtered_tasks = snapshot.filtered_tasks();
    let active_channel = match active_view {
        ActiveView::ChannelDesign => Some(ChannelKind::Design),
        ActiveView::ChannelEngineering => Some(ChannelKind::Engineering),
        _ => None,
    };
    let channel_messages = match active_channel {
        Some(ChannelKind::Engineering) => snapshot.current.eng_messages.clone(),
        Some(ChannelKind::Design) => snapshot.current.design_messages.clone(),
        None => Vec::new(),
    };

    let undo_opacity = if can_undo { "1.0" } else { "0.4" };
    let redo_opacity = if can_redo { "1.0" } else { "0.4" };

    let s_new_task = Arc::clone(&store);
    let s_toggle_studio = Arc::clone(&store);
    let s_xray = Arc::clone(&store);
    let s_tt_first = Arc::clone(&store);
    let s_tt_undo = Arc::clone(&store);
    let s_tt_redo = Arc::clone(&store);
    let s_tt_live = Arc::clone(&store);
    let s_stress8 = Arc::clone(&store);
    let s_stress24 = Arc::clone(&store);
    let s_stress_rst = Arc::clone(&store);

    rsx! {
        view {
            style: "width: 100%; height: 100%; flex-direction: row; background-color: {CANVAS_BG};",

            // =========================================================================
            // 1. LEFT SIDEBAR (172px fixed)
            // =========================================================================
            view {
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
                    for &(view_kind, tid, label, count) in nav_items.iter() {
                        {
                            let is_active = active_view == view_kind;
                            let (bw, bc) = control_border(xray, is_active, accent_hex);
                            let bg = if is_active { accent_tint } else { CONTROL_BG };
                            let fg = if is_active { TEXT_PRIMARY } else { TEXT_SECONDARY };
                            let count_bg = if is_active { accent_hex } else { "#334155" };
                            let count_fg = if is_active { accent.on_accent_fg() } else { TEXT_SECONDARY };
                            let tab_style = format!(
                                "width: 100%; height: 30px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 9px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
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
                                    onpress: move |_| mutate_state(&s_nav, version, |st| st.select_view(view_kind)),
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

            // =========================================================================
            // 2. CENTER MAIN WORKSPACE (flex: 1)
            // =========================================================================
            view {
                style: "{center_style}",

                // Top Header Bar
                view {
                    style: "{header_bar_style}",
                    text {
                        style: "color: {TEXT_PRIMARY}; font-size: 13px;",
                        "{active_view.title()}"
                    }
                    view {
                        style: "{status_pill_style}",
                        text {
                            style: "color: {accent_hex}; font-size: 10px;",
                            "{status_pill_label}"
                        }
                    }
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
                                let is_sel = snapshot.current.board_filter == flt;
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

                    // Scrollable Task Card List
                    scrollview {
                        style: "width: 100%; flex: 1; flex-direction: column; gap: {m.list_gap}px; border-width: {row_bw}px; border-color: {row_bc};",
                        for task in filtered_tasks.iter() {
                            {
                                let tid = task.id;
                                let status_tid = format!("task-status-{tid}");
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
                                        // Assignee & Tag row
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

                // ---- CHANNEL VIEWS (# engineering & # design-sync) ----
                if let Some(ch) = active_channel {
                    scrollview {
                        style: "width: 100%; flex: 1; flex-direction: column; gap: {m.list_gap}px; border-width: {row_bw}px; border-color: {row_bc};",
                        for msg in channel_messages.iter() {
                            {
                                let mid = msg.id;
                                let react_tid = format!("msg-react-{mid}");
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
                                                style: "flex-direction: row; align-items: center; gap: 7px;",
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

                    // Quick-Composer Bar
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 6px; padding: 9px; background-color: {PANEL_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
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
                                        "flex: 1; height: 28px; justify-content: center; align-items: center; background-color: {CARD_BG}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
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
            // 3. RIGHT DOCKED NVE STUDIO META-WORKBENCH (210px when open)
            // =========================================================================
            if studio.panel_open {
                view {
                    test_id: "studio-panel",
                    style: "{studio_panel_style}",

                    // Studio Header
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; border-width: {row_bw}px; border-color: {row_bc};",
                        text {
                            style: "color: {TEXT_PRIMARY}; font-size: 12px;",
                            "NVE STUDIO"
                        }
                        view {
                            style: "height: 18px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 6px; background-color: {accent_tint}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {accent_hex};",
                            text {
                                style: "color: {accent_hex}; font-size: 9px;",
                                "LIVE SELF-EDIT"
                            }
                        }
                    }

                    // Accent Swatches (2x2)
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 4px; padding: 7px; background-color: {CARD_ALT_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                        text { style: "color: {TEXT_MUTED}; font-size: 10px;", "ACCENT" }
                        view {
                            style: "width: 100%; flex-direction: row; gap: 5px;",
                            for &(theme, tid) in [
                                (AccentTheme::Cyan, "accent-cyan"),
                                (AccentTheme::Violet, "accent-violet"),
                            ].iter() {
                                {
                                    let sel = accent == theme;
                                    let swatch = theme.hex();
                                    let bg = if sel { swatch } else { CONTROL_BG };
                                    let fg = if sel { theme.on_accent_fg() } else { swatch };
                                    let (bw, bc) = control_border(xray, sel, swatch);
                                    let s_acc = Arc::clone(&store);
                                    rsx! {
                                        pressable {
                                            key: "{tid}",
                                            test_id: "{tid}",
                                            style: "flex: 1; height: 24px; justify-content: center; align-items: center; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                            onpress: move |_| mutate_state(&s_acc, version, |st| st.set_accent(theme)),
                                            text { style: "color: {fg}; font-size: 11px;", "{theme.label()}" }
                                        }
                                    }
                                }
                            }
                        }
                        view {
                            style: "width: 100%; flex-direction: row; gap: 5px;",
                            for &(theme, tid) in [
                                (AccentTheme::Green, "accent-green"),
                                (AccentTheme::Amber, "accent-amber"),
                            ].iter() {
                                {
                                    let sel = accent == theme;
                                    let swatch = theme.hex();
                                    let bg = if sel { swatch } else { CONTROL_BG };
                                    let fg = if sel { theme.on_accent_fg() } else { swatch };
                                    let (bw, bc) = control_border(xray, sel, swatch);
                                    let s_acc = Arc::clone(&store);
                                    rsx! {
                                        pressable {
                                            key: "{tid}",
                                            test_id: "{tid}",
                                            style: "flex: 1; height: 24px; justify-content: center; align-items: center; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                            onpress: move |_| mutate_state(&s_acc, version, |st| st.set_accent(theme)),
                                            text { style: "color: {fg}; font-size: 11px;", "{theme.label()}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Corner Radius + Density Card
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 5px; padding: 7px; background-color: {CARD_ALT_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                        text { style: "color: {TEXT_MUTED}; font-size: 10px;", "RADIUS (6 / 12 / 18)" }
                        view {
                            style: "width: 100%; flex-direction: row; gap: 4px;",
                            for &(rmode, tid, short_lbl) in [
                                (CornerRadiusMode::Sharp, "radius-sharp", "Sharp"),
                                (CornerRadiusMode::Soft, "radius-soft", "Soft"),
                                (CornerRadiusMode::Pill, "radius-pill", "Pill"),
                            ].iter() {
                                {
                                    let sel = studio.radius == rmode;
                                    let bg = if sel { accent_tint } else { CONTROL_BG };
                                    let fg = if sel { accent_hex } else { TEXT_SECONDARY };
                                    let (bw, bc) = control_border(xray, sel, accent_hex);
                                    let s_rad = Arc::clone(&store);
                                    rsx! {
                                        pressable {
                                            key: "{tid}",
                                            test_id: "{tid}",
                                            style: "flex: 1; height: 24px; justify-content: center; align-items: center; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                            onpress: move |_| mutate_state(&s_rad, version, |st| st.set_radius(rmode)),
                                            text { style: "color: {fg}; font-size: 10px;", "{short_lbl}" }
                                        }
                                    }
                                }
                            }
                        }

                        text { style: "color: {TEXT_MUTED}; font-size: 10px;", "DENSITY" }
                        view {
                            style: "width: 100%; flex-direction: row; gap: 5px;",
                            for &(dmode, tid) in [
                                (DensityMode::Normal, "density-normal"),
                                (DensityMode::Compact, "density-compact"),
                            ].iter() {
                                {
                                    let sel = studio.density == dmode;
                                    let bg = if sel { accent_tint } else { CONTROL_BG };
                                    let fg = if sel { accent_hex } else { TEXT_SECONDARY };
                                    let (bw, bc) = control_border(xray, sel, accent_hex);
                                    let s_den = Arc::clone(&store);
                                    rsx! {
                                        pressable {
                                            key: "{tid}",
                                            test_id: "{tid}",
                                            style: "flex: 1; height: 24px; justify-content: center; align-items: center; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                            onpress: move |_| mutate_state(&s_den, version, |st| st.set_density(dmode)),
                                            text { style: "color: {fg}; font-size: 10px;", "{dmode.label()}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // X-Ray HUD Toggle
                    {
                        let xray_bg = if xray { "#0c2d42" } else { CONTROL_BG };
                        let xray_fg = if xray { "#38bdf8" } else { TEXT_PRIMARY };
                        let xray_bc = if xray { "#38bdf8" } else { "#334155" };
                        let xray_lbl = if xray { "X-Ray HUD: ON" } else { "X-Ray HUD: OFF" };
                        rsx! {
                            pressable {
                                test_id: "toggle-xray",
                                style: "width: 100%; height: 28px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {xray_bg}; border-radius: {m.button_radius}px; border-width: 1px; border-color: {xray_bc};",
                                onpress: move |_| mutate_state(&s_xray, version, |st| st.toggle_xray()),
                                text {
                                    style: "color: {xray_fg}; font-size: 11px;",
                                    "{xray_lbl}"
                                }
                            }
                        }
                    }

                    // Deterministic Time-Travel Scrubber
                    view {
                        style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 5px; padding: 7px; background-color: {CARD_ALT_BG}; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                        text { style: "color: {TEXT_MUTED}; font-size: 10px;", "TIME TRAVEL" }
                        text {
                            style: "color: {TEXT_PRIMARY}; font-size: 12px;",
                            "Snapshot {snap_idx} / {hist_len}"
                        }
                        text {
                            style: "color: {accent_hex}; font-size: 10px;",
                            "{action_label}"
                        }
                        view {
                            style: "width: 100%; flex-direction: row; gap: 5px;",
                            pressable {
                                test_id: "tt-first",
                                style: "flex: 1; height: 24px; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: #334155; opacity: {undo_opacity};",
                                onpress: move |_| mutate_state(&s_tt_first, version, |st| st.scrub_first()),
                                text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "<< #1" }
                            }
                            pressable {
                                test_id: "tt-undo",
                                style: "flex: 1; height: 24px; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: #334155; opacity: {undo_opacity};",
                                onpress: move |_| mutate_state(&s_tt_undo, version, |st| st.scrub_undo()),
                                text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "< Undo" }
                            }
                        }
                        view {
                            style: "width: 100%; flex-direction: row; gap: 5px;",
                            pressable {
                                test_id: "tt-redo",
                                style: "flex: 1; height: 24px; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: #334155; opacity: {redo_opacity};",
                                onpress: move |_| mutate_state(&s_tt_redo, version, |st| st.scrub_redo()),
                                text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "Redo >" }
                            }
                            pressable {
                                test_id: "tt-live",
                                style: "flex: 1; height: 24px; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: #334155; opacity: {redo_opacity};",
                                onpress: move |_| mutate_state(&s_tt_live, version, |st| st.scrub_live()),
                                text { style: "color: {TEXT_PRIMARY}; font-size: 10px;", "Live >>" }
                            }
                        }
                    }
                }
            }
        }
    }
}
