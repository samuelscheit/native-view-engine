//! Issue Board view (`ActiveView::Board`) built on shared `nve-ui` primitives (`TabsRoot`,
//! `TabList`, `TabTrigger`, `Input`, `Button`, `Card`, `Badge`) with live search, custom issue
//! composer, and scrollable task cards with PNG avatar `Image`s.

use std::sync::{Arc, Mutex};

use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Input, TabList,
    TabTrigger, TabsRoot,
};

use crate::assets::avatar_for_author;
use crate::routes::StudioRoute;
use crate::state::{BoardFilter, Priority, TaskItem, WorkspaceSnapshot, WorkspaceState};
use crate::theme::{
    card_border, control_border, row_border, LayoutMetrics, BORDER_DEFAULT, CARD_BG, CONTROL_BG,
    PANEL_BG, STRETCH_COL, TEXT_MUTED, TEXT_PRIMARY, XRAY_ROW,
};
use crate::ui::mutate_state;

fn filter_key(filter: BoardFilter) -> &'static str {
    match filter {
        BoardFilter::All => "all",
        BoardFilter::Active => "active",
        BoardFilter::Done => "done",
    }
}

fn parse_filter(val: &str) -> BoardFilter {
    match val {
        "active" => BoardFilter::Active,
        "done" => BoardFilter::Done,
        _ => BoardFilter::All,
    }
}

fn priority_badge_variant(priority: Priority) -> BadgeVariant {
    match priority {
        Priority::P0 => BadgeVariant::Danger,
        Priority::P1 => BadgeVariant::Warning,
        Priority::P2 => BadgeVariant::Accent,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_board_view(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    current: &WorkspaceSnapshot,
    filtered_tasks: &[TaskItem],
    focused_task_id: Option<u32>,
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

    let s_filter = Arc::clone(store);
    let s_board_search = Arc::clone(store);
    let s_board_search_clear = Arc::clone(store);
    let s_board_draft = Arc::clone(store);
    let s_board_draft_submit = Arc::clone(store);
    let s_board_draft_btn = Arc::clone(store);

    let active_filter_val = filter_key(current.board_filter).to_string();
    let all_lbl = format!("All ({total_tasks})");
    let active_lbl = format!("Active ({active_tasks})");
    let done_lbl = format!("Done ({done_tasks})");

    let tab_list_style = format!(
        "width: 100%; flex-shrink: 0; gap: 6px; padding: 0px; background-color: transparent; border-width: {row_bw}px; border-color: {row_bc}; border-radius: 0px;"
    );
    let clear_btn_style = format!(
        "width: 48px; height: 34px; min-height: 34px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {CONTROL_BG}; border-radius: {}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
        m.button_radius
    );
    let add_btn_style = format!(
        "width: 56px; height: 34px; min-height: 34px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {accent_tint}; border-radius: {}px; border-width: 1px; border-color: {accent_hex};",
        m.button_radius
    );

    rsx! {
        // Filter Tabs Row (`nve-ui` TabsRoot / TabList / TabTrigger)
        TabsRoot {
            value: Some(active_filter_val),
            id_prefix: Some("board-filter".into()),
            style: Some("width: 100%; flex-shrink: 0; gap: 0px;".into()),
            on_change: move |next: String| {
                let flt = parse_filter(&next);
                mutate_state(&s_filter, version, |st| st.set_board_filter(flt));
            },
            TabList {
                accessibility_label: Some("Board issue filter".into()),
                style: Some(tab_list_style),
                for &(flt, tid, ref lbl, pill_w) in [
                    (BoardFilter::All, "filter-all", all_lbl, 68u32),
                    (BoardFilter::Active, "filter-active", active_lbl, 78u32),
                    (BoardFilter::Done, "filter-done", done_lbl, 70u32),
                ].iter() {
                    {
                        let is_sel = current.board_filter == flt;
                        let (bw, bc) = control_border(xray, is_sel, accent_hex);
                        let bg = if is_sel { accent_tint } else { PANEL_BG };
                        let trig_style = format!(
                            "width: {pill_w}px; height: 26px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {bg}; border-radius: {}px; border-width: {bw}px; border-color: {bc};",
                            m.pill_radius
                        );
                        let val_str = filter_key(flt).to_string();
                        rsx! {
                            TabTrigger {
                                key: "{tid}",
                                value: val_str,
                                test_id: Some(tid.into()),
                                style: Some(trig_style),
                                "{lbl}"
                            }
                        }
                    }
                }
            }
        }

        // Live Board Search & Custom Task Create Bar (`nve-ui` Input + Button)
        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 5px; border-width: {row_bw}px; border-color: {row_bc};",
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 5px; align-items: center;",
                view {
                    style: "flex: 1;",
                    Input {
                        test_id: Some("board-search-input".into()),
                        placeholder: Some("Filter issues...".into()),
                        value: current.board_search_query.clone(),
                        accessibility_label: Some("Filter board issues".into()),
                        on_change: move |q: String| {
                            mutate_state(&s_board_search, version, |st| st.set_board_search_query(q));
                        },
                    }
                }
                if !current.board_search_query.is_empty() {
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("board-search-clear".into()),
                        style: Some(clear_btn_style),
                        on_press: move |_| mutate_state(&s_board_search_clear, version, |st| st.set_board_search_query("")),
                        "Clear"
                    }
                }
            }
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 5px; align-items: center;",
                view {
                    style: "flex: 1;",
                    Input {
                        test_id: Some("board-new-task-input".into()),
                        placeholder: Some("New custom issue title...".into()),
                        value: current.new_task_draft.clone(),
                        accessibility_label: Some("New issue title".into()),
                        on_change: move |txt: String| {
                            mutate_state(&s_board_draft, version, |st| st.set_new_task_draft(txt));
                        },
                        on_submit: move |_| {
                            mutate_state(&s_board_draft_submit, version, |st| {
                                let _ = st.submit_new_task_draft();
                            });
                        },
                    }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("board-new-task-submit".into()),
                    style: Some(add_btn_style),
                    on_press: move |_| mutate_state(&s_board_draft_btn, version, |st| {
                        let _ = st.submit_new_task_draft();
                    }),
                    "+ Add"
                }
            }
        }

        // Scrollable Task Card List (`nve-ui` Card + Badge + Button + PNG Image avatars)
        scrollview {
            style: "width: 100%; flex: 1; flex-direction: column; gap: {m.list_gap}px; border-width: {row_bw}px; border-color: {row_bc};",
            for task in filtered_tasks.iter() {
                {
                    let tid = task.id;
                    let is_focused = focused_task_id == Some(tid);
                    let status_tid = format!("task-status-{tid}");
                    let open_tid = format!("task-open-{tid}");
                    let avatar_tid = format!("task-avatar-{tid}");
                    let avatar_src = avatar_for_author(&task.assignee);
                    let task_bc = if is_focused { accent_hex } else { card_bc };
                    let task_bw = if is_focused { 2 } else { card_bw };
                    let card_style = format!(
                        "width: 100%; flex-shrink: 0; padding: {}px; gap: {}px; background-color: {CARD_BG}; border-radius: {}px; border-width: {task_bw}px; border-color: {task_bc};",
                        m.card_pad, m.card_gap, m.card_radius
                    );
                    let p_bg = task.priority.badge_bg();
                    let p_bc = task.priority.badge_border();
                    let p_lbl = task.priority.label();
                    let p_style = format!(
                        "width: 30px; height: 20px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {p_bg}; border-radius: {}px; border-width: 1px; border-color: {p_bc};",
                        m.pill_radius
                    );
                    let st_bg = task.status.pill_bg();
                    let st_bc = if xray { XRAY_ROW } else { task.status.pill_border() };
                    let st_lbl = task.status.label();
                    let st_btn_style = format!(
                        "width: 92px; height: 24px; min-height: 24px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {st_bg}; border-radius: {}px; border-width: 1px; border-color: {st_bc};",
                        m.pill_radius
                    );
                    let meta_line = format!("{} | {}", task.assignee, task.tag);
                    let s_cyc = Arc::clone(store);
                    let s_open = Arc::clone(store);
                    rsx! {
                        Card {
                            key: "task-{tid}",
                            variant: CardVariant::Elevated,
                            padding: Some(m.card_pad as f32),
                            gap: Some(m.card_gap as f32),
                            style: Some(card_style),
                            // Top row: Key + Priority Badge | Clickable Status Button
                            view {
                                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; border-width: {row_bw}px; border-color: {row_bc};",
                                pressable {
                                    test_id: "{open_tid}",
                                    style: "flex-direction: row; align-items: center; gap: 6px;",
                                    onpress: move |_| mutate_state(&s_open, version, |st| {
                                        let _ = st.navigate_to(StudioRoute::Task { id: tid });
                                    }),
                                    Badge {
                                        label: p_lbl.to_string(),
                                        variant: priority_badge_variant(task.priority),
                                        style: Some(p_style),
                                    }
                                    text {
                                        style: "color: {TEXT_MUTED}; font-size: 11px;",
                                        "{task.key}"
                                    }
                                }
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Sm,
                                    test_id: Some(status_tid),
                                    style: Some(st_btn_style),
                                    on_press: move |_| mutate_state(&s_cyc, version, |st| st.cycle_task_status(tid)),
                                    "{st_lbl}"
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
