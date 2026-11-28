//! Sticky right "On This Page" table of contents (`accessibility_role: "complementary"`, `220px`, hidden `< 1200pt`).

use nve_dioxus::prelude::*;
use nve_ui::accent_indicator;

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
            nve_style: TypedStyle::new()
                .width_px(220.0)
                .height_pct(100.0)
                .flex_shrink(0.0)
                .flex_col()
                .padding(20.0)
                .gap(10.0)
                .bg(theme.surface.canvas)
                .border(1.0, theme.border.default),

            text {
                test_id: "docs-toc-title",
                nve_style: TypedStyle::new().font_size(12.0).font_weight(700).color(theme.text.primary),
                "On This Page"
            }

            if headings.is_empty() {
                text {
                    nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                    "No sections"
                }
            } else {
                view {
                    nve_style: TypedStyle::new().flex_col().gap(4.0),
                    for heading in headings {
                        {
                            let is_active = active_anchor.as_deref() == Some(heading.id.as_str());
                            let fg = if is_active { theme.text.link } else { theme.text.secondary };
                            let rail = if is_active { accent_indicator(&theme) } else { theme.border.default };
                            let weight = if is_active { 600 } else { 400 };
                            let pad_h = if heading.level == 3 { 16.0 } else { 8.0 };
                            let href = format!("#{}", heading.id);
                            let tid = format!("docs-toc-{}", heading.id);
                            let anchor_id = heading.id.clone();

                            rsx! {
                                pressable {
                                    key: "{heading.id}",
                                    test_id: "{tid}",
                                    accessibility_role: "link",
                                    "href": "{href}",
                                    nve_style: TypedStyle::new()
                                        .width_pct(100.0)
                                        .flex_row()
                                        .align_items(FlexAlign::Center)
                                        .gap(8.0)
                                        .padding_xy(pad_h, 4.0)
                                        .radius(4.0),
                                    onpress: move |_| {
                                        navigate_to_target(state, None, Some(anchor_id.clone()));
                                    },
                                    view {
                                        nve_style: TypedStyle::new().width_px(2.0).height_px(14.0).bg(rail).radius(1.0),
                                    }
                                    text {
                                        nve_style: TypedStyle::new().font_size(12.0).font_weight(weight).color(fg),
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
