//! Inline issue detail, `<ActionForm<RenameIssue>>`, and comments panel for `IssuesApp`.

use dioxus_core::EventHandler;
use nve_dioxus::prelude::*;
use nve_example_issues_api::{
    AddComment, Comment, Issue, ListComments, RenameIssue,
};
use nve_query::QueryOptions;
use nve_query_dioxus::prelude::*;
use nve_ui::prelude::*;

use crate::state_badge;

#[component]
pub(crate) fn IssueDetailPanel(
    issue: Issue,
    on_dismiss: EventHandler<()>,
    on_renamed: EventHandler<Issue>,
) -> Element {
    let theme = use_theme();
    let p = &theme.palette;
    let fg_hex = p.text_primary.as_ref();
    let muted_hex = p.text_secondary.as_ref();
    let border_hex = p.border.as_ref();
    let subtle_bg = p.surface_subtle.as_ref();
    let id_num = issue.id.0;
    let (badge_label, badge_variant) = state_badge(issue.state);

    let comments_query = use_query_with::<ListComments>(
        issue.id,
        QueryOptions::default().stale_time_ms(30_000),
    );
    let comments: Vec<Comment> = comments_query
        .data()
        .map(|arc| (*arc).clone())
        .unwrap_or_default();

    rsx! {
        Card {
            test_id: "issue-detail-panel".to_owned(),
            variant: CardVariant::Elevated,
            padding: Some(18.0),
            gap: Some(14.0),

            Row {
                justify: Some("space-between".to_owned()),
                Row {
                    gap: Some(8.0),
                    text {
                        style: "font-size: 16px; font-weight: 700; color: {fg_hex};",
                        "Issue #{id_num}"
                    }
                    Badge {
                        label: badge_label.to_owned(),
                        variant: badge_variant,
                    }
                }
                Button {
                    test_id: "issue-detail-close-btn".to_owned(),
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    on_press: move |_| on_dismiss.call(()),
                    "Done"
                }
            }

            ActionForm::<RenameIssue> {
                test_id: "rename-issue-form".to_owned(),
                action: "/issues/rename".to_owned(),
                persist_key: Some(format!("nve-issues-rename-{id_num}")),
                on_success: move |updated: Issue| {
                    on_renamed.call(updated);
                },
                FormHiddenField {
                    name: "id".to_owned(),
                    value: id_num.to_string(),
                }
                FormField {
                    name: "title".to_owned(),
                    test_id: "rename-title-input".to_owned(),
                    label: "Title".to_owned(),
                    initial_value: Some(issue.title.clone()),
                    required: true,
                }
                Row {
                    justify: Some("flex-end".to_owned()),
                    FormSubmitButton {
                        test_id: "rename-submit-btn".to_owned(),
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Sm,
                        "Save Title"
                    }
                }
            }

            Separator {}

            Col {
                gap: Some(8.0),
                text {
                    style: "font-size: 13px; font-weight: 600; color: {muted_hex};",
                    "Comments ({comments.len()})"
                }
                if comments_query.is_loading() {
                    text {
                        style: "font-size: 13px; color: {muted_hex};",
                        "Loading comments..."
                    }
                } else if comments.is_empty() {
                    text {
                        test_id: "comments-empty",
                        style: "font-size: 13px; color: {muted_hex};",
                        "No comments yet."
                    }
                } else {
                    for comment in comments {
                        view {
                            key: "{comment.id.0}",
                            test_id: "comment-row-{comment.id.0}",
                            style: "width: 100%; padding: 10px; border-radius: 8px; border-width: 1px; border-color: {border_hex}; background: {subtle_bg}; display: flex; flex-direction: column; gap: 4px;",
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

            ActionForm::<AddComment> {
                test_id: "add-comment-form".to_owned(),
                action: "/issues/comment".to_owned(),
                persist_key: Some(format!("nve-issues-comment-{id_num}")),
                reset_on_success: true,
                FormHiddenField {
                    name: "issue_id".to_owned(),
                    value: id_num.to_string(),
                }
                FormField {
                    name: "body".to_owned(),
                    test_id: "comment-body-input".to_owned(),
                    label: "Add Comment".to_owned(),
                    placeholder: "Share an update...".to_owned(),
                    required: true,
                }
                Row {
                    justify: Some("flex-end".to_owned()),
                    FormSubmitButton {
                        test_id: "comment-submit-btn".to_owned(),
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        "Post Comment"
                    }
                }
            }
        }
    }
}
