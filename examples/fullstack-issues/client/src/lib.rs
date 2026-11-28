//! Cross-platform fullstack issue-tracker UI (`IssuesApp`) built purely on `nve-query-dioxus`
//! and `nve-ui` with zero platform-specific code.
//!
//! Runs identically over `nve-http-web::FetchTransport` (Web WASM cookie auth),
//! `nve-http-hyper::HyperTransport::with_background_runtime()` (Native Skia Bearer auth),
//! and `nve-server::DirectTransport` (SSR / headless tests).

/// The issues family's web table plugins (`nve-web-html` only: no canvas), registered identically
/// by the SSR server and the browser shell so Frame-0 tags and placement checks match.
#[cfg(feature = "web")]
pub fn register_web_systems(registry: &mut nve_backend_web::WebTableRegistry) {
    nve_web_html_web::register_web_html_systems(registry);
}

mod detail;
mod panels;

use dioxus_core::EventHandler;
use nve_dioxus::prelude::*;
use nve_example_issues_api::{
    CloseIssue, CloseIssueInput, CreateIssue, Issue, IssueFilter, IssueId, IssueState, ListIssues,
};
use nve_query::QueryOptions;
use nve_query_dioxus::prelude::*;
use nve_ui::prelude::*;
use nve_ui::with_alpha;

use crate::detail::IssueDetailPanel;
use crate::panels::{FeaturedCommentsSection, WorkspaceStatsSection};

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
/// [`ThemeProvider()`] and [`QueryClientProvider()`].
#[component]
pub fn IssuesApp(
    client: ErasedQueryClient,
    #[props(default)] sleep: Option<SleepHandle>,
    #[props(default = IssueId(1))] featured_issue_id: IssueId,
) -> Element {
    rsx! {
        ThemeProvider {
            theme: Theme::DARK,
            QueryClientProvider {
                client: client,
                sleep: sleep,
                IssuesWorkspace {
                    featured_issue_id: featured_issue_id,
                }
            }
        }
    }
}

/// Interactive issue-tracker workspace (must be mounted inside [`QueryClientProvider()`] and
/// [`ThemeProvider()`]).
#[component]
pub fn IssuesWorkspace(#[props(default = IssueId(1))] featured_issue_id: IssueId) -> Element {
    let theme = use_theme();
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

    let bg = theme.surface.subtle;
    let surface = theme.surface.base;
    let border = theme.border.default;
    let fg = theme.text.primary;
    let muted = theme.text.secondary;
    let danger = theme.status.danger.text;

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
            nve_style: TypedStyle::new()
                .flex_1()
                .width_pct(100.0)
                .height_pct(100.0)
                .overflow_y(Overflow::Auto)
                .bg(bg)
                .padding(16.0)
                .align_items(FlexAlign::Center),
            view {
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .max_width_px(880.0)
                    .flex_col()
                    .align_items(FlexAlign::Stretch)
                    .gap(16.0),

                // Header row
                Row {
                    justify: Some("space-between".to_owned()),
                    nve_style: Some(TypedStyle::new().flex_wrap(FlexWrap::Wrap).gap(12.0)),
                    Row {
                        gap: Some(10.0),
                        text {
                            test_id: "issues-heading",
                            nve_style: TypedStyle::new().font_size(22.0).font_weight(700).color(fg),
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

                // Streaming workspace summary counters (`<QuerySuspense id="stats">`)
                WorkspaceStatsSection {}

                // Filter tabs
                Row {
                    gap: Some(8.0),
                    nve_style: Some(TypedStyle::new().flex_wrap(FlexWrap::Wrap)),
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
                        nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(muted),
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
                            nve_style: Some(
                                TypedStyle::new()
                                    .flex_wrap(FlexWrap::Wrap)
                                    .width_pct(100.0),
                            ),
                            view {
                                nve_style: TypedStyle::new()
                                    .flex_grow(1.0)
                                    .flex_shrink(1.0)
                                    .flex_basis_px(280.0)
                                    .min_width_px(200.0),
                                FormField {
                                    name: "title".to_owned(),
                                    test_id: "create-title-input".to_owned(),
                                    placeholder: "New issue title...".to_owned(),
                                    required: true,
                                    nve_style: Some(TypedStyle::new().width_pct(100.0)),
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
                        nve_style: TypedStyle::new()
                            .padding(12.0)
                            .radius(8.0)
                            .border(1.0, border)
                            .bg(surface),
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).color(fg),
                            "{banner}"
                        }
                    }
                }

                if let Some(ref err_msg) = query_err_text {
                    view {
                        test_id: "issues-query-error",
                        nve_style: TypedStyle::new()
                            .padding(12.0)
                            .radius(8.0)
                            .border(1.0, danger)
                            .bg(surface)
                            .flex_row()
                            .justify_content(FlexJustify::SpaceBetween)
                            .align_items(FlexAlign::Center)
                            .gap(12.0),
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).color(danger),
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
                        nve_style: TypedStyle::new()
                            .padding(12.0)
                            .radius(8.0)
                            .border(1.0, danger)
                            .bg(surface),
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).color(danger),
                            "Close failed (rolled back): {err_msg}"
                        }
                    }
                }

                // Issue list surface
                view {
                    test_id: "issues-list",
                    nve_style: TypedStyle::new()
                        .width_pct(100.0)
                        .flex_col()
                        .align_items(FlexAlign::Stretch)
                        .border(1.0, border)
                        .radius(10.0)
                        .bg(surface)
                        .overflow_mode(Overflow::Hidden),
                    if issues_query.is_loading() {
                        view {
                            test_id: "issues-loading",
                            nve_style: TypedStyle::new().padding(24.0),
                            text {
                                nve_style: TypedStyle::new().font_size(14.0).color(muted),
                                "Loading issues..."
                            }
                        }
                    } else if issues_list.is_empty() {
                        view {
                            test_id: "issues-empty",
                            nve_style: TypedStyle::new().padding(24.0),
                            text {
                                nve_style: TypedStyle::new().font_size(14.0).color(muted),
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

                // Streaming featured discussion (`<QuerySuspense id="comments">`)
                FeaturedCommentsSection {
                    issue_id: featured_issue_id,
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
    let fg = theme.text.primary;
    let muted = theme.text.secondary;
    let selected_bg = if is_selected {
        theme.state.selected
    } else {
        with_alpha(theme.state.selected, 0)
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
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_col()
                .align_items(FlexAlign::Stretch),
            if show_divider {
                Separator {}
            }
            view {
                test_id: "issue-row-{id_num}",
                nve_style: TypedStyle::new()
                    .width_pct(100.0)
                    .flex_row()
                    .flex_wrap(FlexWrap::Wrap)
                    .align_items(FlexAlign::Center)
                    .justify_content(FlexJustify::SpaceBetween)
                    .gap(12.0)
                    .padding(14.0)
                    .bg(selected_bg),
            Col {
                gap: Some(6.0),
                align: Some("flex-start".to_owned()),
                nve_style: Some(
                    TypedStyle::new()
                        .flex_grow(1.0)
                        .flex_shrink(1.0)
                        .flex_basis(Dimension::Points(260.0))
                        .min_width_px(0.0),
                ),
                Row {
                    gap: Some(8.0),
                    nve_style: Some(TypedStyle::new().flex_wrap(FlexWrap::Wrap)),
                    text {
                        nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(muted),
                        "#{id_num}"
                    }
                    text {
                        test_id: "issue-title-{id_num}",
                        nve_style: TypedStyle::new().font_size(15.0).font_weight(600).color(fg),
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
                        nve_style: TypedStyle::new().font_size(12.0).color(muted),
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
