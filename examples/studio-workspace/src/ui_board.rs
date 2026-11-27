//! Issue Board view (`ActiveView::Board`) built on shared `nve-ui` primitives (`TabsRoot`,
//! `TabList`, `TabTrigger`, `Input`, `Button`, `Card`, `Badge`) with live search, custom issue
//! composer, and scrollable task cards with PNG avatar `Image`s.
//!
//! [`BoardView`] subscribes to the `board` domain and [`BoardComposer`] to `board_draft`, so
//! typing re-renders the composer only; each [`TaskCard`] is memoized on its task so a status
//! change re-renders one card.

use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, ColorHex, Input,
    TabList, TabTrigger, TabsRoot,
};

use crate::assets::avatar_for_author;
use crate::routes::StudioRoute;
use crate::state::{BoardFilter, Priority, TaskItem, TaskStatus, filter_tasks};
use crate::theme::{STRETCH_COL, task_status_tone};
use crate::ui::store::use_studio;
use crate::ui::style::{StudioStyle, use_studio_style};

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

/// Inline style of a 34px board action button.
fn board_button_style(s: &StudioStyle, width: u32, active: bool) -> String {
    format!(
        "width: {width}px; height: 34px; min-height: 34px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {}; border-radius: {}px; {}",
        s.control_bg(active),
        s.m.button_radius,
        s.control(active)
    )
}

/// New-issue composer: the only board part subscribed to `board_draft`.
#[component]
fn BoardComposer() -> Element {
    nve_dioxus::render_probe::record_render("BoardComposer");
    let store = use_studio();
    let s = use_studio_style();
    let draft = store.board_draft.read().new_task_draft.clone();
    rsx! {
        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 5px; align-items: center;",
            view {
                style: "flex: 1;",
                Input {
                    test_id: Some("board-new-task-input".into()),
                    placeholder: Some("New custom issue title...".into()),
                    value: draft,
                    accessibility_label: Some("New issue title".into()),
                    on_change: move |txt: String| store.update(|st| st.set_new_task_draft(txt)),
                    on_submit: move |_| store.update(|st| {
                        let _ = st.submit_new_task_draft();
                    }),
                }
            }
            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("board-new-task-submit".into()),
                style: Some(board_button_style(&s, 56, true)),
                on_press: move |_| store.update(|st| {
                    let _ = st.submit_new_task_draft();
                }),
                "+ Add"
            }
        }
    }
}

#[component]
pub(crate) fn BoardView() -> Element {
    nve_dioxus::render_probe::record_render("BoardView");
    let store = use_studio();
    let s = use_studio_style();
    let board = store.board.read();
    let filter = board.board_filter;
    let search = board.board_search_query.clone();
    let focused = board.focused_task_id;
    let total = board.tasks.len();
    let done = board
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Done)
        .count();
    let visible: Vec<TaskItem> = filter_tasks(&board.tasks, filter, &search)
        .cloned()
        .collect();
    drop(board);

    let row = s.row();

    rsx! {
        TabsRoot {
            value: Some(filter_key(filter).to_string()),
            id_prefix: Some("board-filter".into()),
            style: Some("width: 100%; flex-shrink: 0; gap: 0px;".into()),
            on_change: move |next: String| {
                let flt = parse_filter(&next);
                store.update(|st| st.set_board_filter(flt));
            },
            TabList {
                accessibility_label: Some("Board issue filter".into()),
                style: Some(format!("width: 100%; flex-shrink: 0; gap: 6px; padding: 0px; background-color: transparent; border-radius: 0px; {row}")),
                for (flt, tid, lbl, pill_w) in [
                    (BoardFilter::All, "filter-all", format!("All ({total})"), 68u32),
                    (BoardFilter::Active, "filter-active", format!("Active ({})", total - done), 78u32),
                    (BoardFilter::Done, "filter-done", format!("Done ({done})"), 70u32),
                ] {
                    TabTrigger {
                        key: "{tid}",
                        value: filter_key(flt).to_string(),
                        test_id: Some(tid.into()),
                        style: Some(format!(
                            "width: {pill_w}px; height: 26px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {}; border-radius: {}px; {}",
                            if filter == flt { s.t.status.accent.subtle.hex() } else { s.t.surface.base.hex() },
                            s.m.pill_radius,
                            s.control(filter == flt)
                        )),
                        "{lbl}"
                    }
                }
            }
        }

        view {
            style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 5px; {row}",
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; gap: 5px; align-items: center;",
                view {
                    style: "flex: 1;",
                    Input {
                        test_id: Some("board-search-input".into()),
                        placeholder: Some("Filter issues...".into()),
                        value: search.clone(),
                        accessibility_label: Some("Filter board issues".into()),
                        on_change: move |q: String| store.update(|st| st.set_board_search_query(q)),
                    }
                }
                if !search.is_empty() {
                    Button {
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        test_id: Some("board-search-clear".into()),
                        style: Some(board_button_style(&s, 48, false)),
                        on_press: move |_| store.update(|st| st.set_board_search_query("")),
                        "Clear"
                    }
                }
            }
            BoardComposer {}
        }

        scrollview {
            style: "width: 100%; flex: 1; flex-direction: column; gap: {s.m.list_gap}px; {row}",
            for task in visible {
                TaskCard { key: "task-{task.id}", focused: focused == Some(task.id), task }
            }
        }
    }
}

/// One issue card; re-renders only when its task, focus, or the style changes.
#[component]
fn TaskCard(task: TaskItem, focused: bool) -> Element {
    nve_dioxus::render_probe::record_render("TaskCard");
    let store = use_studio();
    let s = use_studio_style();
    let tid = task.id;
    let card = s.card();
    let (border_w, border_c) = if focused {
        (2, s.t.status.accent.text)
    } else {
        (card.width, card.color)
    };
    let card_style = format!(
        "width: 100%; flex-shrink: 0; padding: {}px; gap: {}px; background-color: {}; border-radius: {}px; border-width: {border_w}px; border-color: {};",
        s.m.card_pad,
        s.m.card_gap,
        s.t.surface.raised.hex(),
        s.m.card_radius,
        border_c.hex()
    );
    let p_style = format!(
        "width: 30px; height: 20px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; border-radius: {}px;",
        s.m.pill_radius
    );
    // Status pill: tinted tone fill + tone border; X-Ray outlines it like any row.
    let tone = task_status_tone(&s.t, task.status);
    let st_border = if s.xray() { s.row().color } else { tone.border };
    let st_btn_style = format!(
        "width: 92px; height: 24px; min-height: 24px; flex-shrink: 0; {STRETCH_COL} padding-horizontal: 2px; padding-vertical: 0px; background-color: {}; border-radius: {}px; border-width: 1px; border-color: {};",
        tone.subtle.hex(),
        s.m.pill_radius,
        st_border.hex()
    );
    let meta_line = format!("{} | {}", task.assignee, task.tag);
    let avatar_src = avatar_for_author(&task.assignee);
    let muted = s.t.text.tertiary.hex();

    rsx! {
        Card {
            variant: CardVariant::Elevated,
            padding: Some(s.m.card_pad as f32),
            gap: Some(s.m.card_gap as f32),
            style: Some(card_style),
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; {s.row()}",
                pressable {
                    test_id: "task-open-{tid}",
                    style: "flex-direction: row; align-items: center; gap: 6px;",
                    onpress: move |_| store.update(|st| {
                        let _ = st.navigate_to(StudioRoute::Task { id: tid });
                    }),
                    Badge {
                        label: task.priority.label().to_string(),
                        variant: priority_badge_variant(task.priority),
                        style: Some(p_style),
                    }
                    text { style: "color: {muted}; font-size: 11px;", "{task.key}" }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some(format!("task-status-{tid}")),
                    style: Some(st_btn_style),
                    on_press: move |_| store.update(|st| st.cycle_task_status(tid)),
                    "{task.status.label()}"
                }
            }
            text { style: "color: {s.t.text.primary.hex()}; font-size: 12px;", "{task.title}" }
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; align-items: center; gap: 6px;",
                image {
                    test_id: "task-avatar-{tid}",
                    src: "{avatar_src}",
                    alt: "{task.assignee}",
                    resize_mode: "cover",
                    accessibility_label: "Assignee avatar {task.assignee}",
                    style: "width: 18px; height: 18px; flex-shrink: 0; border-radius: 9px; border-width: 1px; border-color: {s.t.border.default.hex()};",
                }
                text { style: "color: {muted}; font-size: 11px;", "{meta_line}" }
            }
        }
    }
}
