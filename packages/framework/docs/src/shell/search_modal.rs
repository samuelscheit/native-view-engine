//! `⌘K` instant full-text search dialog built on `nve_ui::Dialog`, `FocusTrap`, and `nve_mdx::SearchIndex`.

use nve_dioxus::prelude::*;
use nve_ui::accent_indicator;
use nve_ui::{Dialog, FocusTrap, FocusTrapAction};

use crate::state::{navigate_to_target, use_docs_state};

const MAX_SEARCH_HITS: usize = 10;

/// Modal `⌘K` search palette backed by the compiled `NVS2` inverted + trigram search index.
#[component]
pub fn SearchModal() -> Element {
    let mut state = use_docs_state();
    let snap = state.read();
    let is_open = snap.search_open;
    let theme = snap.theme();
    let query = snap.search_query.clone();
    let selected_idx = snap.search_selected;
    let config = snap.config.clone();
    let hits = if is_open {
        snap.search_hits(MAX_SEARCH_HITS)
    } else {
        Vec::new()
    };
    drop(snap);

    let mut focusable_ids = Vec::with_capacity(hits.len() + 2);
    focusable_ids.push("docs-search-input".to_string());
    for idx in 0..hits.len() {
        focusable_ids.push(format!("docs-search-hit-{idx}"));
    }
    focusable_ids.push("docs-search-close".to_string());

    let mut local_trap = use_signal(|| {
        FocusTrap::from_ids(["docs-search-input".to_string(), "docs-search-close".to_string()])
            .with_close_on_escape(true)
    });

    if is_open {
        let mut t = local_trap.write_unchecked();
        t.scope_mut().set_items(
            focusable_ids
                .iter()
                .cloned()
                .map(nve_ui::FocusableEntry::new)
                .collect(),
        );
        if !t.is_open() {
            t.activate(Some("docs-search-trigger".to_string()));
        }
    } else if local_trap.peek().is_open() {
        local_trap.write_unchecked().deactivate();
    }

    let hit_count = hits.len();
    let hits_for_key = hits.clone();
    let hits_for_input_key = hits.clone();

    rsx! {
        Dialog {
            open: is_open,
            title: Some("Search Documentation".to_string()),
            description: Some("Type to filter pages and headings. Use ↑/↓ and Enter to jump.".to_string()),
            focusable_ids: focusable_ids,
            previous_focus: Some("docs-search-trigger".to_string()),
            close_on_escape: true,
            close_on_backdrop: true,
            test_id: Some("docs-search-dialog".to_string()),
            nve_style: is_open.then(|| {
                TypedStyle::new()
                    .min_width_px(340.0)
                    .max_width_px(600.0)
                    .width_pct(100.0)
            }),
            on_close: move |_| {
                state.write().close_search();
            },

            view {
                test_id: "docs-search-body",
                nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(12.0),
                onkeydown: move |ev: KeyEvent| {
                    let data = ev.data();
                    if matches!(
                        local_trap.write().handle_key(&data.key, data.shift),
                        FocusTrapAction::Dismiss { .. }
                    ) {
                        state.write().close_search();
                        return;
                    }
                    handle_search_nav_key(state, local_trap, &data, &hits_for_key);
                },

                // Live query input
                view {
                    nve_style: TypedStyle::new().width_pct(100.0).flex_row().align_items(FlexAlign::Center).gap(8.0),
                    textinput {
                        test_id: "docs-search-input",
                        accessibility_role: "searchbox",
                        accessibility_label: "Search documentation query",
                        placeholder: "Search pages, headings, APIs...",
                        value: "{query}",
                        nve_style: TypedStyle::new()
                            .flex_grow(1.0)
                            .padding_xy(12.0, 10.0)
                            .radius(8.0)
                            .border(1.0, accent_indicator(&theme))
                            .bg(theme.surface.canvas)
                            .color(theme.text.primary)
                            .font_size(14.0),
                        onchange: move |ev: TextEvent| {
                            state.write().set_search_query(ev.data().value.clone());
                        },
                        onkeydown: move |ev: KeyEvent| {
                            handle_search_nav_key(state, local_trap, &ev.data(), &hits_for_input_key);
                        },
                        onsubmit: move |_| {
                            let idx = state.read().search_selected;
                            let current_hits = state.read().search_hits(MAX_SEARCH_HITS);
                            if let Some(hit) = current_hits.get(idx).or_else(|| current_hits.first()) {
                                let route = state.read().config.docs_route_for_content(&hit.route);
                                let anchor = (!hit.anchor.is_empty()).then(|| hit.anchor.clone());
                                navigate_to_target(state, Some(route), anchor);
                            }
                        },
                    }
                    pressable {
                        test_id: "docs-search-close",
                        accessibility_role: "button",
                        accessibility_label: "Close search dialog",
                        nve_style: TypedStyle::new()
                            .padding_xy(10.0, 8.0)
                            .radius(6.0)
                            .bg(theme.surface.subtle)
                            .border(1.0, theme.border.default),
                        onpress: move |_| {
                            state.write().close_search();
                        },
                        text {
                            nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                            "Esc"
                        }
                    }
                }

                // Result list
                view {
                    test_id: "docs-search-results",
                    accessibility_role: "listbox",
                    accessibility_label: "Search results",
                    nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(6.0),

                    if hit_count == 0 {
                        view {
                            test_id: "docs-search-empty",
                            nve_style: TypedStyle::new()
                                .padding(20.0)
                                .radius(8.0)
                                .bg(theme.surface.subtle)
                                .align_items(FlexAlign::Center),
                            text {
                                nve_style: TypedStyle::new().font_size(13.0).color(theme.text.secondary),
                                "No matching results for \"{query}\"."
                            }
                        }
                    } else {
                        for (idx, hit) in hits.into_iter().enumerate() {
                            {
                                let is_selected = idx == selected_idx;
                                let bg = if is_selected { theme.status.accent.subtle } else { theme.surface.subtle };
                                let border_c = if is_selected { accent_indicator(&theme) } else { theme.border.default };
                                let title_c = if is_selected { theme.text.link } else { theme.text.primary };
                                let tid = format!("docs-search-hit-{idx}");
                                let sel_attr = if is_selected { "true" } else { "false" };
                                let hit_route = config.docs_route_for_content(&hit.route);
                                let hit_anchor = hit.anchor.clone();
                                let section_badge = if hit.section.is_empty() || hit.section == hit.title {
                                    hit_route.as_str().to_string()
                                } else {
                                    hit.section.clone()
                                };

                                rsx! {
                                    pressable {
                                        key: "search-hit-{idx}-{hit.route}-{hit.anchor}",
                                        test_id: "{tid}",
                                        accessibility_role: "option",
                                        aria_selected: "{sel_attr}",
                                        nve_style: TypedStyle::new()
                                            .width_pct(100.0)
                                            .flex_col()
                                            .gap(4.0)
                                            .padding_xy(12.0, 10.0)
                                            .radius(8.0)
                                            .bg(bg)
                                            .border(1.0, border_c),
                                        onpress: move |_| {
                                            let anchor = (!hit_anchor.is_empty()).then(|| hit_anchor.clone());
                                            navigate_to_target(state, Some(hit_route.clone()), anchor);
                                        },
                                        view {
                                            nve_style: TypedStyle::new()
                                                .width_pct(100.0)
                                                .flex_row()
                                                .justify_content(FlexJustify::SpaceBetween)
                                                .align_items(FlexAlign::Center)
                                                .gap(8.0),
                                            text {
                                                nve_style: TypedStyle::new().font_size(14.0).font_weight(600).color(title_c),
                                                "{hit.title}"
                                            }
                                            view {
                                                nve_style: TypedStyle::new()
                                                    .padding_xy(6.0, 2.0)
                                                    .radius(4.0)
                                                    .bg(theme.surface.base)
                                                    .border(1.0, theme.border.default),
                                                text {
                                                    nve_style: TypedStyle::new().font_size(11.0).color(theme.status.info.text),
                                                    "{section_badge}"
                                                }
                                            }
                                        }
                                        if !hit.excerpt.is_empty() {
                                            text {
                                                nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                                                "{hit.excerpt}"
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
    }
}

fn handle_search_nav_key(
    mut state: crate::state::DocsState,
    mut local_trap: Signal<FocusTrap>,
    data: &KeyEventData,
    hit_list: &[nve_mdx::SearchHit],
) {
    match data.key.as_str() {
        "ArrowDown" => {
            state.write().move_search_selection(1, hit_list.len());
        }
        "ArrowUp" => {
            state.write().move_search_selection(-1, hit_list.len());
        }
        "Enter" => {
            let idx = state.read().search_selected;
            if let Some(hit) = hit_list.get(idx).or_else(|| hit_list.first()) {
                let route = state.read().config.docs_route_for_content(&hit.route);
                let anchor = (!hit.anchor.is_empty()).then(|| hit.anchor.clone());
                navigate_to_target(state, Some(route), anchor);
            }
        }
        "Escape" => {
            state.write().close_search();
        }
        "Tab" => {
            let _ = local_trap.write().handle_key(&data.key, data.shift);
        }
        _ => {}
    }
}
