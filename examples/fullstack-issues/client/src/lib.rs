//! Cross-platform fullstack issue-tracker UI (`IssuesApp`) built purely on `nve-query-dioxus`
//! and `nve-ui` with zero platform-specific code.
//!
//! Runs identically over `nve-http-web::FetchTransport` (Web WASM cookie auth),
//! `nve-http-hyper::HyperTransport::with_background_runtime()` (Native Skia Bearer auth),
//! and `nve-server::DirectTransport` (SSR / headless tests).

mod detail;

use dioxus_core::EventHandler;
use nve_dioxus::prelude::*;
use nve_example_issues_api::{
    CloseIssue, CloseIssueInput, CreateIssue, Issue, IssueFilter, IssueId, IssueState, ListIssues,
};
use nve_query::QueryOptions;
use nve_query_dioxus::prelude::*;
use nve_ui::prelude::*;

use crate::detail::IssueDetailPanel;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FilterTab {
    All,
    Open,
    InProgress,
    Closed,
}

impl FilterTab {
    const ALL: [Self; 4] = [Self::All, Self::Open, Self::InProgress, Self::Closed];

    const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Open => "Open",
            Self::InProgress => "In Progress",
            Self::Closed => "Closed",
        }
    }

    const fn test_id(self) -> &'static str {
        match self {
            Self::All => "filter-all",
            Self::Open => "filter-open",
            Self::InProgress => "filter-in-progress",
            Self::Closed => "filter-closed",
        }
    }

    const fn to_state(self) -> Option<IssueState> {
        match self {
            Self::All => None,
            Self::Open => Some(IssueState::Open),
            Self::InProgress => Some(IssueState::InProgress),
            Self::Closed => Some(IssueState::Closed),
        }
    }
}

pub(crate) fn state_badge(state: IssueState) -> (&'static str, BadgeVariant) {
    match state {
        IssueState::Open => ("Open", BadgeVariant::Accent),
        IssueState::InProgress => ("In Progress", BadgeVariant::Warning),
        IssueState::Closed => ("Closed", BadgeVariant::Success),
        IssueState::Unknown(_) => ("Unknown", BadgeVariant::Neutral),
    }
}

/// Top-level portable issue-tracker application wrapping [`IssuesWorkspace`] in
/// [`ThemeProvider`] and [`QueryClientProvider`].
#[component]
pub fn IssuesApp(
    client: ErasedQueryClient,
    #[props(default)] sleep: Option<SleepHandle>,
) -> Element {
    rsx! {
        ThemeProvider {
            theme: UiTheme::dark(),
            QueryClientProvider {
                client: client,
                sleep: sleep,
                IssuesWorkspace {}
            }
        }
    }
}

/// Interactive issue-tracker workspace (must be mounted inside [`QueryClientProvider`] and
/// [`ThemeProvider`]).
#[component]
pub fn IssuesWorkspace() -> Element {
    let theme = use_theme();
    let p = &theme.palette;
    let mut active_tab = use_signal(|| FilterTab::All);
    let mut selected_id = use_signal(|| Option::<IssueId>::None);
    let mut status_banner = use_signal(|| Option::<String>::None);

    let filter = IssueFilter {
        state: active_tab().to_state(),
        query: None,
    };
    let issues_query = use_query_with::<ListIssues>(
        filter,
        QueryOptions::default()
            .stale_time_ms(30_000)
            .keep_previous_data(true),
    );
    let close_action = use_action::<CloseIssue>();

    let issues_data = issues_query.data();
    let issues_list: Vec<Issue> = issues_data
        .as_ref()
        .map(|arc| (**arc).clone())
        .unwrap_or_default();
    let selected_issue = selected_id()
        .and_then(|id| issues_list.iter().find(|it| it.id == id).cloned());

    let bg_hex = p.surface_subtle.as_ref();
    let surface_hex = p.surface.as_ref();
    let border_hex = p.border.as_ref();
    let fg_hex = p.text_primary.as_ref();
    let muted_hex = p.text_secondary.as_ref();
    let danger_hex = p.danger.as_ref();

    let count_label = if issues_list.len() == 1 {
        "1 issue".to_owned()
    } else {
        format!("{} issues", issues_list.len())
    };
    let fetch_status = if issues_query.is_fetching() {
        "Syncing..."
    } else {
        "Synced"
    };
    let query_err_text = issues_query.error().map(|e| e.to_string());
    let close_err_text = close_action.error().map(|e| e.to_string());

    rsx! {
        scrollview {
            test_id: "issues-root",
            style: "flex: 1; width: 100%; height: 100%; overflow-y: auto; background: {bg_hex}; padding: 16px; align-items: center;",
            view {
                style: "width: 100%; max-width: 880px; display: flex; flex-direction: column; align-items: stretch; gap: 16px;",

                // Header row
                Row {
                    justify: Some("space-between".to_owned()),
                    style: Some("flex-wrap: wrap; gap: 12px;".to_owned()),
                    Row {
                        gap: Some(10.0),
                        text {
                            test_id: "issues-heading",
                            style: "font-size: 22px; font-weight: 700; color: {fg_hex};",
                            "Issue Tracker"
                        }
                        Badge {
                            test_id: "issues-count-badge",
                            label: count_label,
                            variant: BadgeVariant::Neutral,
                        }
                        Badge {
                            test_id: "issues-sync-badge",
                            label: fetch_status.to_owned(),
                            variant: if issues_query.is_fetching() {
                                BadgeVariant::Warning
                            } else {
                                BadgeVariant::Success
                            },
                        }
                    }
                    Button {
                        test_id: "issues-refresh-btn",
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        on_press: move |_| {
                            issues_query.refetch();
                        },
                        "Refresh"
                    }
                }

                // Filter tabs
                Row {
                    gap: Some(8.0),
                    style: Some("flex-wrap: wrap;".to_owned()),
                    for tab in FilterTab::ALL {
                        Button {
                            key: "{tab.test_id()}",
                            test_id: tab.test_id().to_owned(),
                            variant: if active_tab() == tab {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Ghost
                            },
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                active_tab.set(tab);
                            },
                            "{tab.label()}"
                        }
                    }
                }

                // Create Issue progressive ActionForm
                Card {
                    variant: CardVariant::Outlined,
                    padding: Some(16.0),
                    gap: Some(10.0),
                    text {
                        style: "font-size: 13px; font-weight: 600; color: {muted_hex};",
                        "Create Issue"
                    }
                    ActionForm::<CreateIssue> {
                        test_id: "create-issue-form".to_owned(),
                        action: "/issues/create".to_owned(),
                        persist_key: Some("nve-issues-create-draft".to_owned()),
                        reset_on_success: true,
                        on_success: move |created: Issue| {
                            status_banner.set(Some(format!(
                                "Created #{}: {}",
                                created.id.0, created.title
                            )));
                            selected_id.set(Some(created.id));
                        },
                        Row {
                            gap: Some(10.0),
                            align: Some("flex-start".to_owned()),
                            style: Some("flex-wrap: wrap; width: 100%;".to_owned()),
                            view {
                                style: "flex: 1 1 280px; min-width: 200px;",
                                FormField {
                                    name: "title".to_owned(),
                                    test_id: "create-title-input".to_owned(),
                                    placeholder: "New issue title...".to_owned(),
                                    required: true,
                                    style: Some("width: 100%;".to_owned()),
                                }
                            }
                            FormSubmitButton {
                                test_id: "create-submit-btn".to_owned(),
                                variant: ButtonVariant::Primary,
                                size: ButtonSize::Md,
                                "Create Issue"
                            }
                        }
                    }
                }

                if let Some(ref banner) = status_banner() {
                    view {
                        test_id: "issues-status-banner",
                        style: "padding: 12px; border-radius: 8px; border: 1px solid {border_hex}; background: {surface_hex};",
                        text {
                            style: "font-size: 13px; color: {fg_hex};",
                            "{banner}"
                        }
                    }
                }

                if let Some(ref err_msg) = query_err_text {
                    view {
                        test_id: "issues-query-error",
                        style: "padding: 12px; border-radius: 8px; border: 1px solid {danger_hex}; background: {surface_hex}; display: flex; justify-content: space-between; align-items: center; gap: 12px;",
                        text {
                            style: "font-size: 13px; color: {danger_hex};",
                            "Failed to load issues: {err_msg}"
                        }
                        Button {
                            test_id: "issues-retry-btn",
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            on_press: move |_| issues_query.refetch(),
                            "Retry"
                        }
                    }
                }

                if let Some(ref err_msg) = close_err_text {
                    view {
                        test_id: "issues-close-error",
                        style: "padding: 12px; border-radius: 8px; border: 1px solid {danger_hex}; background: {surface_hex};",
                        text {
                            style: "font-size: 13px; color: {danger_hex};",
                            "Close failed (rolled back): {err_msg}"
                        }
                    }
                }

                // Issue list surface
                view {
                    test_id: "issues-list",
                    style: "width: 100%; display: flex; flex-direction: column; align-items: stretch; border-width: 1px; border-color: {border_hex}; border-radius: 10px; background: {surface_hex}; overflow: hidden;",
                    if issues_query.is_loading() {
                        view {
                            test_id: "issues-loading",
                            style: "padding: 24px;",
                            text {
                                style: "font-size: 14px; color: {muted_hex};",
                                "Loading issues..."
                            }
                        }
                    } else if issues_list.is_empty() {
                        view {
                            test_id: "issues-empty",
                            style: "padding: 24px;",
                            text {
                                style: "font-size: 14px; color: {muted_hex};",
                                "No issues match this filter."
                            }
                        }
                    } else {
                        for (idx, issue) in issues_list.iter().enumerate() {
                            IssueRowItem {
                                key: "{issue.id.0}",
                                issue: issue.clone(),
                                is_selected: selected_id() == Some(issue.id),
                                show_divider: idx > 0,
                                on_select: move |id| {
                                    selected_id.set(Some(id));
                                },
                                on_close: move |id: IssueId| {
                                    status_banner.set(Some(format!("Closing issue #{}...", id.0)));
                                    close_action.run_optimistic(
                                        CloseIssueInput { id },
                                        move |tx| {
                                            tx.update_each::<ListIssues>(move |list| {
                                                list.iter()
                                                    .map(|item| {
                                                        if item.id == id {
                                                            let mut next = item.clone();
                                                            next.state = IssueState::Closed;
                                                            next
                                                        } else {
                                                            item.clone()
                                                        }
                                                    })
                                                    .collect()
                                            });
                                        },
                                    );
                                },
                            }
                        }
                    }
                }

                // Selected issue inline editor + comments
                if let Some(issue) = selected_issue {
                    IssueDetailPanel {
                        key: "{issue.id.0}",
                        issue: issue,
                        on_dismiss: move |_| selected_id.set(None),
                        on_renamed: move |updated: Issue| {
                            status_banner.set(Some(format!(
                                "Renamed #{} to {}",
                                updated.id.0, updated.title
                            )));
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn IssueRowItem(
    issue: Issue,
    is_selected: bool,
    show_divider: bool,
    on_select: EventHandler<IssueId>,
    on_close: EventHandler<IssueId>,
) -> Element {
    let theme = use_theme();
    let p = &theme.palette;
    let fg_hex = p.text_primary.as_ref();
    let muted_hex = p.text_secondary.as_ref();
    let selected_bg = if is_selected {
        p.surface_elevated.as_ref()
    } else {
        "transparent"
    };
    let (badge_text, badge_variant) = state_badge(issue.state);
    let id = issue.id;
    let id_num = id.0;
    let is_closed = issue.state == IssueState::Closed;
    let comments_label = if issue.comment_count == 1 {
        "1 comment".to_owned()
    } else {
        format!("{} comments", issue.comment_count)
    };

    rsx! {
        view {
            style: "width: 100%; display: flex; flex-direction: column; align-items: stretch;",
            if show_divider {
                Separator {}
            }
            view {
                test_id: "issue-row-{id_num}",
                style: "width: 100%; display: flex; flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px; padding: 14px; background: {selected_bg};",
            Col {
                gap: Some(6.0),
                align: Some("flex-start".to_owned()),
                style: Some("flex: 1 1 260px; min-width: 0;".to_owned()),
                Row {
                    gap: Some(8.0),
                    style: Some("flex-wrap: wrap;".to_owned()),
                    text {
                        style: "font-size: 12px; font-weight: 600; color: {muted_hex};",
                        "#{id_num}"
                    }
                    text {
                        test_id: "issue-title-{id_num}",
                        style: "font-size: 15px; font-weight: 600; color: {fg_hex};",
                        "{issue.title}"
                    }
                }
                Row {
                    gap: Some(8.0),
                    Badge {
                        test_id: "issue-state-{id_num}",
                        label: badge_text.to_owned(),
                        variant: badge_variant,
                    }
                    text {
                        test_id: "issue-comments-count-{id_num}",
                        style: "font-size: 12px; color: {muted_hex};",
                        "{comments_label}"
                    }
                }
            }
            Row {
                gap: Some(8.0),
                Button {
                    test_id: "issue-select-{id_num}",
                    variant: if is_selected {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    },
                    size: ButtonSize::Sm,
                    on_press: move |_| on_select.call(id),
                    if is_selected { "Editing" } else { "Edit" }
                }
                if !is_closed {
                    Button {
                        test_id: "issue-close-{id_num}",
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Sm,
                        on_press: move |_| on_close.call(id),
                        "Close"
                    }
                }
            }
            }
        }
    }
}
