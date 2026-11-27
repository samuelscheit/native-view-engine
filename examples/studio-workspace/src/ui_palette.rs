//! `Cmd+K` Command Palette with deterministic fuzzy filtering, keyboard navigation
//! (`ArrowUp`/`ArrowDown`/`Enter`/`Escape`), and click execution across views, tasks,
//! accent themes, layout engines, and time-travel snapshots.

use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, ColorHex, Dialog, Input, Separator,
};
use serde::{Deserialize, Serialize};

use crate::state::{AccentTheme, ActiveView, LayoutEngineMode, WorkspaceState};
use crate::theme::badge_min_w;
use crate::ui::store::use_studio;
use crate::ui::style::use_studio_style;

/// Maximum number of filtered palette results rendered concurrently so the overlay remains compact.
pub const MAX_VISIBLE_PALETTE_ITEMS: usize = 7;

/// Transient command palette state (preserved across snapshots/reloads without polluting time-travel).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommandPaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
}

/// Keyboard event descriptor for command palette and workspace shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyDownEvent {
    pub key: String,
    pub meta: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyDownEvent {
    pub fn has_primary_modifier(&self) -> bool {
        self.meta || self.ctrl
    }
}

/// Executable action represented by one Command Palette row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPaletteAction {
    SelectView(ActiveView),
    CycleTask(u32),
    SetAccent(AccentTheme),
    SetLayoutEngine(LayoutEngineMode),
    ScrubSnapshot(usize),
}

/// One searchable entry inside the `Cmd+K` Command Palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPaletteItem {
    pub category: &'static str,
    pub label: String,
    pub subtitle: String,
    pub action: CommandPaletteAction,
}

/// Deterministic fuzzy matcher scoring case-insensitive substring & ordered subsequence hits.
///
/// Returns `Some(score)` where higher is better, or `None` when `query` does not match `candidate`.
#[expect(
    clippy::disallowed_methods,
    reason = "interactive command-palette fuzzy matcher lowercases query and candidate strings"
)]
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let c = candidate.to_ascii_lowercase();

    // Direct contiguous substring match gets the strongest signal.
    if let Some(pos) = c.find(&q) {
        let mut score = 200 - (pos as i32).min(120);
        if pos == 0 {
            score += 60;
        } else {
            let prev = c.as_bytes()[pos - 1];
            if prev == b' ' || prev == b'-' || prev == b'_' || prev == b':' || prev == b'#' {
                score += 35;
            }
        }
        if c.len() == q.len() {
            score += 50;
        }
        return Some(score);
    }

    // Ordered subsequence fuzzy match (e.g. "isld" -> "Islands", "yga" -> "Yoga").
    // Evaluate every candidate start position matching `q_chars[0]` so a tight word-boundary
    // match (e.g. "Yoga" in "Layout Engine: Yoga") always outranks a scattered earlier hit.
    let q_chars: Vec<char> = q.chars().collect();
    let c_chars: Vec<char> = c.chars().collect();
    let is_boundary = |idx: usize| -> bool {
        idx == 0 || matches!(c_chars[idx - 1], ' ' | '-' | '_' | ':' | '#' | '/' | '(')
    };

    let mut best_score: Option<i32> = None;
    for start_ci in 0..c_chars.len() {
        if c_chars[start_ci] != q_chars[0] {
            continue;
        }
        let mut qi = 0usize;
        let mut score = if is_boundary(start_ci) { 30 } else { 10 };
        let mut consecutive = 0i32;
        let mut prev_ci: Option<usize> = None;

        for (ci, &ch) in c_chars.iter().enumerate().skip(start_ci) {
            if qi < q_chars.len() && ch == q_chars[qi] {
                score += 14;
                if qi > 0 && is_boundary(ci) {
                    score += 18;
                }
                if let Some(prev) = prev_ci {
                    let gap = ci - prev - 1;
                    if gap == 0 {
                        consecutive += 1;
                        score += 12 * consecutive;
                    } else {
                        consecutive = 0;
                        score -= (gap as i32 * 2).min(24);
                    }
                }
                prev_ci = Some(ci);
                qi += 1;
                if qi == q_chars.len() {
                    break;
                }
            }
        }

        if qi == q_chars.len() {
            let candidate_score = score.max(1);
            best_score = Some(best_score.map_or(candidate_score, |b| b.max(candidate_score)));
        }
    }

    best_score
}

/// Builds the full live catalog of views, tasks, accent themes, layout engines, and snapshots.
pub fn build_palette_items(state: &WorkspaceState) -> Vec<CommandPaletteItem> {
    let mut items = Vec::with_capacity(24 + state.current.tasks.len());

    // 1. Views
    for (view, label, sub) in [
        (ActiveView::Board, "View: Board", "Issue Board & triage"),
        (
            ActiveView::ChannelEngineering,
            "View: #engineering",
            "Engineering chat channel",
        ),
        (
            ActiveView::ChannelDesign,
            "View: #design-sync",
            "Design sync chat channel",
        ),
        (
            ActiveView::Docs,
            "View: Docs",
            "Block documents & Markdown export",
        ),
        (
            ActiveView::Workbench,
            "View: Workbench",
            "Multi-viewport preview & 9 host surfaces",
        ),
        (
            ActiveView::Stream100k,
            "View: 100k Stream",
            "Virtualized Fenwick 100k stream",
        ),
        (
            ActiveView::Islands,
            "View: Islands",
            "Multi-host RN / Flutter / SwiftUI / Compose",
        ),
        (
            ActiveView::Metrics,
            "View: Metrics",
            "SkiaCanvas sparkline & stress metrics",
        ),
    ] {
        items.push(CommandPaletteItem {
            category: "VIEW",
            label: label.into(),
            subtitle: sub.into(),
            action: CommandPaletteAction::SelectView(view),
        });
    }

    // 2. Live Tasks
    for task in &state.current.tasks {
        items.push(CommandPaletteItem {
            category: "TASK",
            label: format!("{} {}", task.key, task.title),
            subtitle: format!("{} | {} | {}", task.status.label(), task.assignee, task.tag),
            action: CommandPaletteAction::CycleTask(task.id),
        });
    }

    // 3. Accent Colors
    for accent in [
        AccentTheme::Cyan,
        AccentTheme::Violet,
        AccentTheme::Green,
        AccentTheme::Amber,
    ] {
        items.push(CommandPaletteItem {
            category: "ACCENT",
            label: format!("Accent: {}", accent.label()),
            subtitle: format!("Switch the Studio accent to {}", accent.label()),
            action: CommandPaletteAction::SetAccent(accent),
        });
    }

    // 4. Layout Engines
    for engine in [LayoutEngineMode::Taffy, LayoutEngineMode::Yoga] {
        items.push(CommandPaletteItem {
            category: "LAYOUT",
            label: format!("Layout Engine: {}", engine.label()),
            subtitle: engine.readout_label().into(),
            action: CommandPaletteAction::SetLayoutEngine(engine),
        });
    }

    // 5. Recent Time-Travel Snapshots (newest first, capped to 8 entries)
    let start = state.history.len().saturating_sub(8);
    for idx in (start..state.history.len()).rev() {
        let snap = &state.history[idx];
        items.push(CommandPaletteItem {
            category: "HISTORY",
            label: format!("Snapshot #{}: {}", idx + 1, snap.action_label),
            subtitle: format!(
                "Jump to {} ({})",
                snap.active_view.title(),
                snap.studio.accent.label()
            ),
            action: CommandPaletteAction::ScrubSnapshot(idx),
        });
    }

    items
}

/// Fuzzy-filters and ranks all palette items against `state.palette.query`.
pub fn filter_palette_items(state: &WorkspaceState) -> Vec<CommandPaletteItem> {
    let query = state.palette.query.trim();
    let all = build_palette_items(state);
    if query.is_empty() {
        return all;
    }

    let mut scored: Vec<(i32, usize, CommandPaletteItem)> = all
        .into_iter()
        .enumerate()
        .filter_map(|(orig_idx, item)| {
            let haystack = format!("{} {} {}", item.category, item.label, item.subtitle);
            let best = fuzzy_score(query, &item.label)
                .map(|s| s + 25)
                .into_iter()
                .chain(fuzzy_score(query, &haystack))
                .max()?;
            Some((best, orig_idx, item))
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

/// Applies one chosen [`CommandPaletteAction`] to [`WorkspaceState`] and closes the palette.
pub fn execute_palette_action(state: &mut WorkspaceState, action: CommandPaletteAction) {
    state.palette.open = false;
    state.palette.query.clear();
    state.palette.selected = 0;

    match action {
        CommandPaletteAction::SelectView(view) => state.select_view(view),
        CommandPaletteAction::CycleTask(task_id) => {
            if state.current.active_view != ActiveView::Board {
                state.select_view(ActiveView::Board);
            }
            state.cycle_task_status(task_id);
        }
        CommandPaletteAction::SetAccent(accent) => state.set_accent(accent),
        CommandPaletteAction::SetLayoutEngine(mode) => state.set_layout_engine(mode),
        CommandPaletteAction::ScrubSnapshot(idx) => state.scrub_to(idx),
    }
}

/// Handles a `keydown` event for the `Cmd+K` palette and keyboard shortcuts.
///
/// Returns `true` when the key mutated state and requires a re-render.
pub fn dispatch_keydown(state: &mut WorkspaceState, key: &str, meta_or_ctrl: bool) -> bool {
    if meta_or_ctrl && key.eq_ignore_ascii_case("k") {
        state.toggle_palette();
        return true;
    }

    if !state.palette.open {
        return false;
    }

    match key {
        "Escape" | "Esc" => {
            state.close_palette();
            true
        }
        "ArrowDown" | "Down" => {
            state.move_palette_selection(1);
            true
        }
        "ArrowUp" | "Up" => {
            state.move_palette_selection(-1);
            true
        }
        "Enter" => state.execute_palette_selected(),
        _ => false,
    }
}
/// The `Cmd+K` overlay; subscribes to the `palette` domain only.
#[component]
pub(crate) fn CommandPalette() -> Element {
    nve_dioxus::render_probe::record_render("CommandPalette");
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let palette = store.palette.read();
    let (open, query, selected) = (
        palette.state.open,
        palette.state.query.clone(),
        palette.state.selected,
    );
    drop(palette);
    let mut visible = store.with(filter_palette_items);
    let total = visible.len();
    visible.truncate(MAX_VISIBLE_PALETTE_ITEMS);
    let selected = selected.min(visible.len().saturating_sub(1));

    let focusable_ids: Vec<String> = ["palette-input".to_owned(), "palette-close".to_owned()]
        .into_iter()
        .chain((0..visible.len()).map(|idx| format!("palette-item-{idx}")))
        .collect();
    let dialog_style = format!(
        "width: 100%; min-width: 0px; max-width: 100%; padding: 8px; gap: 6px; border-radius: {}px;",
        s.m.card_radius
    );

    rsx! {
        Dialog {
            open,
            test_id: Some("command-palette".into()),
            accessibility_label: Some("Command Palette".into()),
            focusable_ids,
            close_on_escape: true,
            close_on_backdrop: true,
            style: Some(dialog_style),
            on_close: move |_| store.update(|st| st.close_palette()),

            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center;",
                view {
                    style: "flex-direction: row; align-items: center; gap: 6px;",
                    text { style: "color: {t.text.link.hex()}; font-size: 11px;", "CMD+K PALETTE" }
                    text { style: "color: {t.text.tertiary.hex()}; font-size: 10px;", "{total} matches" }
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("palette-close".into()),
                    accessibility_label: Some("Close command palette".into()),
                    style: Some(s.compact_control("width: 42px; height: 20px; min-height: 20px;", 2, s.m.pill_radius, Some(false))),
                    on_press: move |_| store.update(|st| st.close_palette()),
                    "Esc"
                }
            }

            Input {
                test_id: Some("palette-input".into()),
                placeholder: Some("Jump to view, task, accent, layout, snapshot...".into()),
                value: query,
                accessibility_label: Some("Command palette filter".into()),
                accessibility_role: Some("searchbox".into()),
                on_change: move |q: String| store.update(|st| st.set_palette_query(q)),
                on_submit: move |_| store.update(|st| {
                    let _ = st.execute_palette_selected();
                }),
            }

            Separator {}

            if visible.is_empty() {
                view {
                    test_id: "palette-empty",
                    style: "width: 100%; height: 26px; justify-content: center; align-items: center; background-color: {t.surface.sunken.hex()}; border-radius: {s.m.pill_radius}px;",
                    text { style: "color: {t.text.tertiary.hex()}; font-size: 10px;", "No matching commands" }
                }
            } else {
                view {
                    style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 4px;",
                    for (idx, item) in visible.into_iter().enumerate() {
                        {
                            let is_sel = idx == selected;
                            let bg = if is_sel { t.status.accent.subtle } else { t.surface.raised };
                            rsx! {
                                pressable {
                                    key: "pal-{idx}",
                                    test_id: "palette-item-{idx}",
                                    style: "width: 100%; height: 26px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 8px; background-color: {bg.hex()}; border-radius: {s.m.pill_radius}px; {s.control(is_sel)}",
                                    onpress: move |_| store.update(|st| {
                                        let _ = st.execute_palette_at(idx);
                                    }),
                                    view {
                                        style: "flex-direction: row; align-items: center; gap: 6px; flex: 1;",
                                        Badge {
                                            label: item.category.to_string(),
                                            variant: BadgeVariant::Neutral,
                                            style: Some(s.compact_control(
                                                &format!("min-width: {}px; height: 16px;", badge_min_w(item.category)),
                                                3,
                                                t.radius.sm as u32,
                                                None,
                                            )),
                                        }
                                        text { style: "color: {s.control_fg(is_sel)}; font-size: 10px;", "{item.label}" }
                                    }
                                    if is_sel {
                                        text { style: "color: {t.text.link.hex()}; font-size: 9px;", "Enter" }
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
