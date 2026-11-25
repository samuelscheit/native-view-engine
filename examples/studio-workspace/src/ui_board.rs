//! Issue Board view (`ActiveView::Board`) with filter pills, live search `TextInput`,
//! custom issue composer, and scrollable task cards with PNG avatar `Image`s.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;

use crate::assets::avatar_for_author;
use crate::state::{BoardFilter, TaskItem, WorkspaceSnapshot, WorkspaceState};
use crate::theme::{
    card_border, control_border, row_border, LayoutMetrics, BORDER_DEFAULT, CARD_BG, CONTROL_BG,
    PANEL_BG, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY, XRAY_ROW,
};
use crate::ui::mutate_state;

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_board_view(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    current: &WorkspaceSnapshot,
    filtered_tasks: &[TaskItem],
    total_tasks: usize,
    active_tasks: usize,
    done_tasks: usize,
    accent_hex: &'static str,
    accent_tint: &'static str,
    xray: bool,
    m: LayoutMetrics,
) -> Element {
    let (card_bw, card_bc) = card_border(xray);
    let (row_bw, row_bc) = row_border(xray);

    let s_board_search = Arc::clone(store);
    let s_board_search_clear = Arc::clone(store);
    let s_board_draft = Arc::clone(store);
    let s_board_draft_submit = Arc::clone(store);
    let s_board_draft_btn = Arc::clone(store);

    rsx! {
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
                    let s_flt = Arc::clone(store);
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
                    let s_cyc = Arc::clone(store);
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
}
