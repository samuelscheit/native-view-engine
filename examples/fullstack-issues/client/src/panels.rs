//! Streaming `<QuerySuspense>` summary and featured-discussion panels for `IssuesApp`.
//!
//! Both panels run identically under SSR (`QueryMode::Ssr` out-of-order Declarative Shadow DOM),
//! Web WASM (mid-stream and post-stream hydration), and Native Skia desktop hosts.

use nve_dioxus::prelude::*;
use nve_dioxus::{ErrorBoundaryReset, UiErrorReport};
use nve_example_issues_api::{
    GetWorkspaceStats, IssueId, ListComments, StatsRequest, WorkspaceStats,
};
use nve_query::QueryOptions;
use nve_query_dioxus::prelude::*;
use nve_ui::prelude::*;

/// Streaming workspace counters wrapped in `<QuerySuspense id="stats">`.
#[component]
pub(crate) fn WorkspaceStatsSection() -> Element {
    let theme = use_theme();
    let border = theme.border.default;
    let surface = theme.surface.base;
    let muted = theme.text.secondary;
    let danger = theme.status.danger.text;

    rsx! {
        QuerySuspense {
            id: Some("stats".to_owned()),
            fallback: rsx! {
                view {
                    test_id: "stats-skeleton",
                    nve_style: TypedStyle::new()
                        .width_pct(100.0)
                        .padding_xy(16.0, 14.0)
                        .radius(10.0)
                        .border(1.0, border)
                        .bg(surface)
                        .flex_row()
                        .align_items(FlexAlign::Center)
                        .justify_content(FlexJustify::SpaceBetween),
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).color(muted),
                        "Loading workspace stats..."
                    }
                }
            },
            error_fallback: move |report: UiErrorReport, reset: ErrorBoundaryReset| {
                let msg = report.message;
                rsx! {
                    view {
                        test_id: "stats-error-boundary",
                        nve_style: TypedStyle::new()
                            .width_pct(100.0)
                            .padding_xy(16.0, 14.0)
                            .radius(10.0)
                            .border(1.0, danger)
                            .bg(surface)
                            .flex_row()
                            .flex_wrap(FlexWrap::Wrap)
                            .align_items(FlexAlign::Center)
                            .justify_content(FlexJustify::SpaceBetween)
                            .gap(12.0),
                        text {
                            nve_style: TypedStyle::new().font_size(13.0).color(danger),
                            "Failed to load workspace stats: {msg}"
                        }
                        Button {
                            test_id: "stats-error-retry",
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            on_press: move |_| reset.reset(),
                            "Retry"
                        }
                    }
                }
            },
            WorkspaceStatsContent {}
        }
    }
}

#[component]
fn WorkspaceStatsContent() -> Element {
    let theme = use_theme();
    let border = theme.border.default;
    let surface = theme.surface.base;
    let fg = theme.text.primary;
    let muted = theme.text.secondary;

    let stats_query = use_query_with::<GetWorkspaceStats>(
        StatsRequest::default(),
        QueryOptions::default().stale_time_ms(30_000),
    );
    let stats_arc = stats_query.suspend()?;
    let stats: WorkspaceStats = *stats_arc;
    let comments_badge = if stats.total_comments == 1 {
        "1 comment".to_owned()
    } else {
        format!("{} comments", stats.total_comments)
    };

    rsx! {
        view {
            test_id: "stats-panel",
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .flex_row()
                .flex_wrap(FlexWrap::Wrap)
                .align_items(FlexAlign::Stretch)
                .gap(10.0),
            StatMetricTile {
                test_id: "stats-total",
                label: "Total",
                value: stats.total.to_string(),
                border,
                surface,
                fg,
                muted,
            }
            StatMetricTile {
                test_id: "stats-open",
                label: "Open",
                value: stats.open.to_string(),
                border,
                surface,
                fg,
                muted,
            }
            StatMetricTile {
                test_id: "stats-in-progress",
                label: "In Progress",
                value: stats.in_progress.to_string(),
                border,
                surface,
                fg,
                muted,
            }
            StatMetricTile {
                test_id: "stats-closed",
                label: "Closed",
                value: stats.closed.to_string(),
                border,
                surface,
                fg,
                muted,
            }
            view {
                test_id: "stats-comments",
                nve_style: TypedStyle::new()
                    .flex_grow(1.0)
                    .flex_shrink(1.0)
                    .flex_basis_px(130.0)
                    .min_width_px(110.0)
                    .padding_xy(14.0, 12.0)
                    .radius(10.0)
                    .border(1.0, border)
                    .bg(surface)
                    .flex_col()
                    .justify_content(FlexJustify::Center)
                    .gap(4.0),
                text {
                    nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(muted),
                    "Discussion"
                }
                text {
                    nve_style: TypedStyle::new().font_size(15.0).font_weight(700).color(fg),
                    "{comments_badge}"
                }
            }
        }
    }
}

#[component]
fn StatMetricTile(
    test_id: &'static str,
    label: &'static str,
    value: String,
    border: ColorRgba,
    surface: ColorRgba,
    fg: ColorRgba,
    muted: ColorRgba,
) -> Element {
    rsx! {
        view {
            test_id: "{test_id}",
            nve_style: TypedStyle::new()
                .flex_grow(1.0)
                .flex_shrink(1.0)
                .flex_basis_px(110.0)
                .min_width_px(96.0)
                .padding_xy(14.0, 12.0)
                .radius(10.0)
                .border(1.0, border)
                .bg(surface)
                .flex_col()
                .gap(4.0),
            text {
                nve_style: TypedStyle::new().font_size(11.0).font_weight(600).color(muted),
                "{label}"
            }
            text {
                nve_style: TypedStyle::new().font_size(20.0).font_weight(700).color(fg),
                "{value}"
            }
        }
    }
}

/// Streaming featured issue comments panel wrapped in `<QuerySuspense id="comments">`.
///
/// When pointed at a missing issue (e.g. `?comments_issue=999`), `ListComments` settles to
/// `ApiError::NotFound` after HTTP `200 OK` headers are flushed and renders the identical
/// `ErrorBoundary` fallback across 0-JS SSR and hydrated WASM.
#[component]
pub(crate) fn FeaturedCommentsSection(issue_id: IssueId) -> Element {
    let theme = use_theme();
    let border = theme.border.default;
    let surface = theme.surface.base;
    let muted = theme.text.secondary;
    let danger = theme.status.danger.text;
    let id_num = issue_id.0;

    rsx! {
        QuerySuspense {
            id: Some("comments".to_owned()),
            fallback: rsx! {
                view {
                    test_id: "comments-skeleton",
                    nve_style: TypedStyle::new()
                        .width_pct(100.0)
                        .padding(16.0)
                        .radius(10.0)
                        .border(1.0, border)
                        .bg(surface)
                        .flex_col()
                        .gap(6.0),
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(muted),
                        "Featured Discussion · Issue #{id_num}"
                    }
                    text {
                        nve_style: TypedStyle::new().font_size(13.0).color(muted),
                        "Streaming comments..."
                    }
                }
            },
            error_fallback: move |report: UiErrorReport, reset: ErrorBoundaryReset| {
                let msg = report.message;
                rsx! {
                    view {
                        test_id: "comments-error-boundary",
                        nve_style: TypedStyle::new()
                            .width_pct(100.0)
                            .padding(16.0)
                            .radius(10.0)
                            .border(1.0, danger)
                            .bg(surface)
                            .flex_col()
                            .gap(10.0),
                        Row {
                            justify: Some("space-between".to_owned()),
                            nve_style: Some(TypedStyle::new().flex_wrap(FlexWrap::Wrap).gap(8.0)),
                            Col {
                                gap: Some(4.0),
                                align: Some("flex-start".to_owned()),
                                text {
                                    nve_style: TypedStyle::new().font_size(13.0).font_weight(700).color(danger),
                                    "Could not load discussion for Issue #{id_num}"
                                }
                                text {
                                    test_id: "comments-error-message",
                                    nve_style: TypedStyle::new().font_size(12.0).color(muted),
                                    "{msg}"
                                }
                            }
                            Button {
                                test_id: "comments-error-retry",
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                on_press: move |_| reset.reset(),
                                "Retry"
                            }
                        }
                    }
                }
            },
            FeaturedCommentsContent { issue_id }
        }
    }
}

#[component]
fn FeaturedCommentsContent(issue_id: IssueId) -> Element {
    let theme = use_theme();
    let border = theme.border.default;
    let surface = theme.surface.base;
    let subtle_bg = theme.surface.subtle;
    let fg = theme.text.primary;
    let muted = theme.text.secondary;
    let id_num = issue_id.0;

    let comments_query = use_query_with::<ListComments>(
        issue_id,
        QueryOptions::default().stale_time_ms(30_000),
    );
    let comments = comments_query.suspend()?;

    rsx! {
        view {
            test_id: "featured-comments-panel",
            nve_style: TypedStyle::new()
                .width_pct(100.0)
                .padding(16.0)
                .radius(10.0)
                .border(1.0, border)
                .bg(surface)
                .flex_col()
                .gap(10.0),
            Row {
                justify: Some("space-between".to_owned()),
                text {
                    nve_style: TypedStyle::new().font_size(13.0).font_weight(600).color(muted),
                    "Featured Discussion · Issue #{id_num}"
                }
                Badge {
                    test_id: "featured-comments-count",
                    label: if comments.len() == 1 {
                        "1 comment".to_owned()
                    } else {
                        format!("{} comments", comments.len())
                    },
                    variant: BadgeVariant::Neutral,
                }
            }
            if comments.is_empty() {
                text {
                    test_id: "featured-comments-empty",
                    nve_style: TypedStyle::new().font_size(13.0).color(muted),
                    "No comments on this issue yet."
                }
            } else {
                for comment in comments.iter() {
                    view {
                        key: "{comment.id.0}",
                        test_id: "featured-comment-{comment.id.0}",
                        nve_style: TypedStyle::new()
                            .width_pct(100.0)
                            .padding_xy(12.0, 10.0)
                            .radius(8.0)
                            .border(1.0, border)
                            .bg(subtle_bg)
                            .flex_col()
                            .gap(4.0),
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).font_weight(600).color(muted),
                            "{comment.author}"
                        }
                        text {
                            nve_style: TypedStyle::new().font_size(14.0).color(fg),
                            "{comment.body}"
                        }
                    }
                }
            }
        }
    }
}
