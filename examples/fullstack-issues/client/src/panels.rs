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
    let p = &theme.palette;
    let border_hex = p.border.as_ref().to_owned();
    let surface_hex = p.surface.as_ref().to_owned();
    let muted_hex = p.text_secondary.as_ref().to_owned();
    let danger_hex = p.danger.as_ref().to_owned();

    let err_border = danger_hex.clone();
    let err_surface = surface_hex.clone();
    let err_text = danger_hex;

    rsx! {
        QuerySuspense {
            id: Some("stats".to_owned()),
            fallback: rsx! {
                view {
                    test_id: "stats-skeleton",
                    style: "width: 100%; padding: 14px 16px; border-radius: 10px; border: 1px solid {border_hex}; background: {surface_hex}; display: flex; flex-direction: row; align-items: center; justify-content: space-between;",
                    text {
                        style: "font-size: 13px; color: {muted_hex};",
                        "Loading workspace stats..."
                    }
                }
            },
            error_fallback: move |report: UiErrorReport, reset: ErrorBoundaryReset| {
                let msg = report.message;
                let b = err_border.clone();
                let s = err_surface.clone();
                let t = err_text.clone();
                rsx! {
                    view {
                        test_id: "stats-error-boundary",
                        style: "width: 100%; padding: 14px 16px; border-radius: 10px; border: 1px solid {b}; background: {s}; display: flex; flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px;",
                        text {
                            style: "font-size: 13px; color: {t};",
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
    let p = &theme.palette;
    let border_hex = p.border.as_ref();
    let surface_hex = p.surface.as_ref();
    let fg_hex = p.text_primary.as_ref();
    let muted_hex = p.text_secondary.as_ref();

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
            style: "width: 100%; display: flex; flex-direction: row; flex-wrap: wrap; align-items: stretch; gap: 10px;",
            StatMetricTile {
                test_id: "stats-total",
                label: "Total",
                value: stats.total.to_string(),
                border_hex: border_hex.to_owned(),
                surface_hex: surface_hex.to_owned(),
                fg_hex: fg_hex.to_owned(),
                muted_hex: muted_hex.to_owned(),
            }
            StatMetricTile {
                test_id: "stats-open",
                label: "Open",
                value: stats.open.to_string(),
                border_hex: border_hex.to_owned(),
                surface_hex: surface_hex.to_owned(),
                fg_hex: fg_hex.to_owned(),
                muted_hex: muted_hex.to_owned(),
            }
            StatMetricTile {
                test_id: "stats-in-progress",
                label: "In Progress",
                value: stats.in_progress.to_string(),
                border_hex: border_hex.to_owned(),
                surface_hex: surface_hex.to_owned(),
                fg_hex: fg_hex.to_owned(),
                muted_hex: muted_hex.to_owned(),
            }
            StatMetricTile {
                test_id: "stats-closed",
                label: "Closed",
                value: stats.closed.to_string(),
                border_hex: border_hex.to_owned(),
                surface_hex: surface_hex.to_owned(),
                fg_hex: fg_hex.to_owned(),
                muted_hex: muted_hex.to_owned(),
            }
            view {
                test_id: "stats-comments",
                style: "flex: 1 1 130px; min-width: 110px; padding: 12px 14px; border-radius: 10px; border: 1px solid {border_hex}; background: {surface_hex}; display: flex; flex-direction: column; justify-content: center; gap: 4px;",
                text {
                    style: "font-size: 11px; font-weight: 600; color: {muted_hex};",
                    "Discussion"
                }
                text {
                    style: "font-size: 15px; font-weight: 700; color: {fg_hex};",
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
    border_hex: String,
    surface_hex: String,
    fg_hex: String,
    muted_hex: String,
) -> Element {
    rsx! {
        view {
            test_id: "{test_id}",
            style: "flex: 1 1 110px; min-width: 96px; padding: 12px 14px; border-radius: 10px; border: 1px solid {border_hex}; background: {surface_hex}; display: flex; flex-direction: column; gap: 4px;",
            text {
                style: "font-size: 11px; font-weight: 600; color: {muted_hex};",
                "{label}"
            }
            text {
                style: "font-size: 20px; font-weight: 700; color: {fg_hex};",
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
    let p = &theme.palette;
    let border_hex = p.border.as_ref().to_owned();
    let surface_hex = p.surface.as_ref().to_owned();
    let muted_hex = p.text_secondary.as_ref().to_owned();
    let danger_hex = p.danger.as_ref().to_owned();
    let id_num = issue_id.0;

    let err_border = danger_hex.clone();
    let err_surface = surface_hex.clone();
    let err_text = danger_hex;
    let err_muted = muted_hex.clone();

    rsx! {
        QuerySuspense {
            id: Some("comments".to_owned()),
            fallback: rsx! {
                view {
                    test_id: "comments-skeleton",
                    style: "width: 100%; padding: 16px; border-radius: 10px; border: 1px solid {border_hex}; background: {surface_hex}; display: flex; flex-direction: column; gap: 6px;",
                    text {
                        style: "font-size: 13px; font-weight: 600; color: {muted_hex};",
                        "Featured Discussion · Issue #{id_num}"
                    }
                    text {
                        style: "font-size: 13px; color: {muted_hex};",
                        "Streaming comments..."
                    }
                }
            },
            error_fallback: move |report: UiErrorReport, reset: ErrorBoundaryReset| {
                let msg = report.message;
                let b = err_border.clone();
                let s = err_surface.clone();
                let t = err_text.clone();
                let m = err_muted.clone();
                rsx! {
                    view {
                        test_id: "comments-error-boundary",
                        style: "width: 100%; padding: 16px; border-radius: 10px; border: 1px solid {b}; background: {s}; display: flex; flex-direction: column; gap: 10px;",
                        Row {
                            justify: Some("space-between".to_owned()),
                            style: Some("flex-wrap: wrap; gap: 8px;".to_owned()),
                            Col {
                                gap: Some(4.0),
                                align: Some("flex-start".to_owned()),
                                text {
                                    style: "font-size: 13px; font-weight: 700; color: {t};",
                                    "Could not load discussion for Issue #{id_num}"
                                }
                                text {
                                    test_id: "comments-error-message",
                                    style: "font-size: 12px; color: {m};",
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
    let p = &theme.palette;
    let border_hex = p.border.as_ref();
    let surface_hex = p.surface.as_ref();
    let subtle_bg = p.surface_subtle.as_ref();
    let fg_hex = p.text_primary.as_ref();
    let muted_hex = p.text_secondary.as_ref();
    let id_num = issue_id.0;

    let comments_query = use_query_with::<ListComments>(
        issue_id,
        QueryOptions::default().stale_time_ms(30_000),
    );
    let comments = comments_query.suspend()?;

    rsx! {
        view {
            test_id: "featured-comments-panel",
            style: "width: 100%; padding: 16px; border-radius: 10px; border: 1px solid {border_hex}; background: {surface_hex}; display: flex; flex-direction: column; gap: 10px;",
            Row {
                justify: Some("space-between".to_owned()),
                text {
                    style: "font-size: 13px; font-weight: 600; color: {muted_hex};",
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
                    style: "font-size: 13px; color: {muted_hex};",
                    "No comments on this issue yet."
                }
            } else {
                for comment in comments.iter() {
                    view {
                        key: "{comment.id.0}",
                        test_id: "featured-comment-{comment.id.0}",
                        style: "width: 100%; padding: 10px 12px; border-radius: 8px; border: 1px solid {border_hex}; background: {subtle_bg}; display: flex; flex-direction: column; gap: 4px;",
                        text {
                            style: "font-size: 12px; font-weight: 600; color: {muted_hex};",
                            "{comment.author}"
                        }
                        text {
                            style: "font-size: 14px; color: {fg_hex};",
                            "{comment.body}"
                        }
                    }
                }
            }
        }
    }
}
