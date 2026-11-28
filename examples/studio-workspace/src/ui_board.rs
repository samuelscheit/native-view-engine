//! Issue Board view (`ActiveView::Board`) built on shared `nve-ui` primitives (`TabsRoot`,
//! `TabList`, `TabTrigger`, `Input`, `Button`, `Card`, `Badge`) with live search, custom issue
//! composer, and scrollable task cards with PNG avatar `Image`s.
//!
//! [`BoardView`] subscribes to the `board` domain and [`BoardComposer`] to `board_draft`, so
//! typing re-renders the composer only; each [`TaskCard`] is memoized on its task so a status
//! change re-renders one card.

use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Input, TabList,
    TabTrigger, TabsRoot,
};

use crate::assets::avatar_for_author;
use crate::routes::StudioRoute;
use crate::state::{BoardFilter, Priority, TaskItem, TaskStatus, filter_tasks};
use crate::theme::task_status_tone;
use crate::ui::store::use_studio;
use crate::ui::style::{
    StudioStyle, StudioStyleExt, fixed_dims, row_between, row_center, text_style, use_studio_style,
};

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

/// Typed style of a 34px board action button.
fn board_button_style(s: &StudioStyle, width: u32, active: bool) -> TypedStyle {
    s.compact_control(
        fixed_dims(width as f32, 34.0),
        2,
        s.m.button_radius,
        Some(active),
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
            nve_style: row_center(5.0).width_pct(100.0).flex_shrink(0.0),
            view {
                nve_style: TypedStyle::new().flex_1(),
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
                nve_style: Some(board_button_style(&s, 56, true)),
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
            nve_style: Some(TypedStyle::new().width_pct(100.0).flex_shrink(0.0).gap(0.0)),
            on_change: move |next: String| {
                let flt = parse_filter(&next);
                store.update(|st| st.set_board_filter(flt));
            },
            TabList {
                accessibility_label: Some("Board issue filter".into()),
                nve_style: Some(
                    TypedStyle::new()
                        .width_pct(100.0)
                        .flex_shrink(0.0)
                        .gap(6.0)
                        .padding(0.0)
                        .bg(ColorRgba::TRANSPARENT)
                        .radius(0.0)
                        .outline(row),
                ),
                for (flt, tid, lbl, pill_w) in [
                    (BoardFilter::All, "filter-all", format!("All ({total})"), 68u32),
                    (BoardFilter::Active, "filter-active", format!("Active ({})", total - done), 78u32),
                    (BoardFilter::Done, "filter-done", format!("Done ({done})"), 70u32),
                ] {
                    TabTrigger {
                        key: "{tid}",
                        value: filter_key(flt).to_string(),
                        test_id: Some(tid.into()),
                        nve_style: Some(
                            TypedStyle::new()
                                .width_px(pill_w as f32)
                                .height_px(26.0)
                                .flex_shrink(0.0)
                                .stretch_col()
                                .padding_xy(2.0, 0.0)
                                .bg(if filter == flt {
                                    s.t.status.accent.subtle
                                } else {
                                    s.t.surface.base
                                })
                                .radius(s.m.pill_radius as f32)
                                .outline(s.control(filter == flt)),
                        ),
                        "{lbl}"
                    }
                }
            }
        }

        view {
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_shrink(0.0)
                .flex_col()
                .gap(5.0)
                .outline(row),
            view {
                nve_style: row_center(5.0).width_pct(100.0).flex_shrink(0.0),
                view {
                    nve_style: TypedStyle::new().flex_1(),
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
                        nve_style: Some(board_button_style(&s, 48, false)),
                        on_press: move |_| store.update(|st| st.set_board_search_query("")),
                        "Clear"
                    }
                }
            }
            BoardComposer {}
        }

        scrollview {
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_1()
                .flex_col()
                .gap(s.m.list_gap as f32)
                .outline(row),
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
        (2.0, s.t.status.accent.text)
    } else {
        (card.width as f32, card.color)
    };
    let card_style = TypedStyle::new()
        .width_pct(100.0)
        .flex_shrink(0.0)
        .padding(s.m.card_pad as f32)
        .gap(s.m.card_gap as f32)
        .bg(s.t.surface.raised)
        .radius(s.m.card_radius as f32)
        .border(border_w, border_c);
    let p_style = TypedStyle::new()
        .width_px(30.0)
        .height_px(20.0)
        .flex_shrink(0.0)
        .stretch_col()
        .padding_xy(2.0, 0.0)
        .radius(s.m.pill_radius as f32);
    // Status pill: tinted tone fill + tone border; X-Ray outlines it like any row.
    let tone = task_status_tone(&s.t, task.status);
    let st_border = if s.xray() { s.row().color } else { tone.border };
    let st_btn_style = s
        .compact_control(fixed_dims(92.0, 24.0), 2, s.m.pill_radius, None)
        .bg(tone.subtle)
        .border(1.0, st_border);
    let meta_line = format!("{} | {}", task.assignee, task.tag);
    let avatar_src = avatar_for_author(&task.assignee);
    let muted = s.t.text.tertiary;

    rsx! {
        Card {
            variant: CardVariant::Elevated,
            padding: Some(s.m.card_pad as f32),
            gap: Some(s.m.card_gap as f32),
            nve_style: Some(card_style),
            view {
                nve_style: row_between().outline(s.row()),
                pressable {
                    test_id: "task-open-{tid}",
                    nve_style: row_center(6.0),
                    onpress: move |_| store.update(|st| {
                        let _ = st.navigate_to(StudioRoute::Task { id: tid });
                    }),
                    Badge {
                        label: task.priority.label().to_string(),
                        variant: priority_badge_variant(task.priority),
                        nve_style: Some(p_style),
                    }
                    text { nve_style: text_style(muted, 11.0), "{task.key}" }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some(format!("task-status-{tid}")),
                    nve_style: Some(st_btn_style),
                    on_press: move |_| store.update(|st| st.cycle_task_status(tid)),
                    "{task.status.label()}"
                }
            }
            text { nve_style: text_style(s.t.text.primary, 12.0), "{task.title}" }
            view {
                nve_style: row_center(6.0).width_pct(100.0).flex_shrink(0.0),
                image {
                    test_id: "task-avatar-{tid}",
                    src: "{avatar_src}",
                    alt: "{task.assignee}",
                    resize_mode: "cover",
                    accessibility_label: "Assignee avatar {task.assignee}",
                    nve_style: TypedStyle::new()
                        .width_px(18.0)
                        .height_px(18.0)
                        .flex_shrink(0.0)
                        .radius(9.0)
                        .border(1.0, s.t.border.default),
                }
                text { nve_style: text_style(muted, 11.0), "{meta_line}" }
            }
        }
    }
}
