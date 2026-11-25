//! Sticky right "On This Page" table of contents (`accessibility_role: "complementary"`, `220px`, hidden `< 1200pt`).

use nve_dioxus::prelude::*;

use crate::state::{navigate_to_target, use_docs_state};

/// Right-rail table of contents showing H2 and H3 headings from the active compiled page.
#[component]
pub fn DocsToc() -> Element {
    let state = use_docs_state();
    let snap = state.read();
    let theme = snap.theme();
    let active_anchor = snap.active_anchor.clone();
    let headings: Vec<_> = snap
        .current_page()
        .map(|page| {
            page.toc
                .iter()
                .filter(|h| h.level == 2 || h.level == 3)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    drop(snap);

    rsx! {
        view {
            test_id: "docs-toc",
            nve_show_on: ShowOn(MediaQuery::min_width(1200)),
            accessibility_role: "complementary",
            accessibility_label: "On this page",
            style: "width: 220px; height: 100%; flex-shrink: 0; flex-direction: column; padding: 20px; gap: 10px; background-color: {theme.canvas}; border-width: 1px; border-color: {theme.border};",

            text {
                test_id: "docs-toc-title",
                style: "font-size: 12px; font-weight: 700; color: {theme.text};",
                "On This Page"
            }

            if headings.is_empty() {
                text {
                    style: "font-size: 12px; color: {theme.muted};",
                    "No sections"
                }
            } else {
                view {
                    style: "flex-direction: column; gap: 4px;",
                    for heading in headings {
                        {
                            let is_active = active_anchor.as_deref() == Some(heading.id.as_str());
                            let fg = if is_active { theme.accent } else { theme.muted };
                            let rail = if is_active { theme.accent } else { theme.border };
                            let weight = if is_active { "600" } else { "400" };
                            let pad_h = if heading.level == 3 { "16px" } else { "8px" };
                            let href = format!("#{}", heading.id);
                            let tid = format!("docs-toc-{}", heading.id);
                            let anchor_id = heading.id.clone();

                            rsx! {
                                pressable {
                                    key: "{heading.id}",
                                    test_id: "{tid}",
                                    accessibility_role: "link",
                                    "href": "{href}",
                                    style: "width: 100%; flex-direction: row; align-items: center; gap: 8px; padding-vertical: 4px; padding-horizontal: {pad_h}; border-radius: 4px;",
                                    onpress: move |_| {
                                        navigate_to_target(state, None, Some(anchor_id.clone()));
                                    },
                                    view {
                                        style: "width: 2px; height: 14px; background-color: {rail}; border-radius: 1px;",
                                    }
                                    text {
                                        style: "font-size: 12px; font-weight: {weight}; color: {fg};",
                                        "{heading.text}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
