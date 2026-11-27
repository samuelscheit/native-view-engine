//! `⌘K` instant full-text search dialog built on `nve_ui::Dialog`, `FocusTrap`, and `nve_mdx::SearchIndex`.

use nve_dioxus::prelude::*;
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
            style: Some(format!(
                "min-width: 340px; max-width: 600px; width: 100%; background-color: {}; border-color: {};",
                theme.surface, theme.border
            )),
            on_close: move |_| {
                state.write().close_search();
            },

            view {
                test_id: "docs-search-body",
                style: "width: 100%; flex-direction: column; gap: 12px;",
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
                    style: "width: 100%; flex-direction: row; align-items: center; gap: 8px;",
                    textinput {
                        test_id: "docs-search-input",
                        accessibility_role: "searchbox",
                        accessibility_label: "Search documentation query",
                        placeholder: "Search pages, headings, APIs...",
                        value: "{query}",
                        style: "flex-grow: 1; padding-horizontal: 12px; padding-vertical: 10px; border-radius: 8px; border-width: 1px; border-color: {theme.accent}; background-color: {theme.canvas}; color: {theme.text}; font-size: 14px;",
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
                        style: "padding-horizontal: 10px; padding-vertical: 8px; border-radius: 6px; background-color: {theme.surface_subtle}; border-width: 1px; border-color: {theme.border};",
                        onpress: move |_| {
                            state.write().close_search();
                        },
                        text {
                            style: "font-size: 12px; color: {theme.muted};",
                            "Esc"
                        }
                    }
                }

                // Result list
                view {
                    test_id: "docs-search-results",
                    accessibility_role: "listbox",
                    accessibility_label: "Search results",
                    style: "width: 100%; flex-direction: column; gap: 6px;",

                    if hit_count == 0 {
                        view {
                            test_id: "docs-search-empty",
                            style: "padding: 20px; border-radius: 8px; background-color: {theme.surface_subtle}; align-items: center;",
                            text {
                                style: "font-size: 13px; color: {theme.muted};",
                                "No matching results for \"{query}\"."
                            }
                        }
                    } else {
                        for (idx, hit) in hits.into_iter().enumerate() {
                            {
                                let is_selected = idx == selected_idx;
                                let bg = if is_selected { theme.accent_surface } else { theme.surface_subtle };
                                let border_c = if is_selected { theme.accent } else { theme.border };
                                let title_c = if is_selected { theme.accent } else { theme.text };
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
                                        style: "width: 100%; flex-direction: column; gap: 4px; padding-horizontal: 12px; padding-vertical: 10px; border-radius: 8px; background-color: {bg}; border-width: 1px; border-color: {border_c};",
                                        onpress: move |_| {
                                            let anchor = (!hit_anchor.is_empty()).then(|| hit_anchor.clone());
                                            navigate_to_target(state, Some(hit_route.clone()), anchor);
                                        },
                                        view {
                                            style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; gap: 8px;",
                                            text {
                                                style: "font-size: 14px; font-weight: 600; color: {title_c};",
                                                "{hit.title}"
                                            }
                                            view {
                                                style: "padding-horizontal: 6px; padding-vertical: 2px; border-radius: 4px; background-color: {theme.surface}; border-width: 1px; border-color: {theme.border};",
                                                text {
                                                    style: "font-size: 11px; color: {theme.accent_alt};",
                                                    "{section_badge}"
                                                }
                                            }
                                        }
                                        if !hit.excerpt.is_empty() {
                                            text {
                                                style: "font-size: 12px; color: {theme.muted};",
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
    mut state: Signal<crate::state::DocsAppState>,
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
