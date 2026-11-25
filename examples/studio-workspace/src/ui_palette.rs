//! `Cmd+K` Command Palette with deterministic fuzzy filtering, keyboard navigation
//! (`ArrowUp`/`ArrowDown`/`Enter`/`Escape`), and click execution across views, tasks,
//! accent themes, layout engines, and time-travel snapshots.

use std::sync::{Arc, Mutex};

use nve_abi::events::decode_keydown_bytes;
use nve_dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::state::{AccentTheme, ActiveView, LayoutEngineMode, WorkspaceState};
use crate::theme::{
    card_border, control_border, LayoutMetrics, BORDER_DEFAULT, CARD_ALT_BG, CARD_BG, CONTROL_BG,
    PANEL_BG, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use crate::ui::mutate_state;

/// Maximum number of filtered palette results rendered concurrently so the overlay remains compact.
pub const MAX_VISIBLE_PALETTE_ITEMS: usize = 7;

/// Transient command palette state (preserved across snapshots/reloads without polluting time-travel).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommandPaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
}

/// Decoded keyboard event arriving via `NativeEventKind::Custom { name: "keydown", .. }`.
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

/// Parses a `keydown` custom event payload from either zero-JSON binary `NK1` framing
/// (`[b'N', b'K', b'1', flags_u8, utf8_key...]`) or compact UTF-8 shorthand
/// (`"Cmd+k"`, `"Ctrl+k"`, `"ArrowDown"`, `"ArrowUp"`, `"Enter"`, `"Escape"`).
pub fn parse_keydown_payload(data: &[u8]) -> Option<KeyDownEvent> {
    let kd = decode_keydown_bytes(data)?;
    Some(KeyDownEvent {
        key: kd.key,
        meta: kd.meta,
        ctrl: kd.ctrl,
        shift: kd.shift,
        alt: kd.alt,
    })
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
        idx == 0
            || matches!(
                c_chars[idx - 1],
                ' ' | '-' | '_' | ':' | '#' | '/' | '('
            )
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
            subtitle: format!("Switch studio accent to {}", accent.hex()),
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
            subtitle: format!("Jump to {} ({})", snap.active_view.title(), snap.studio.accent.label()),
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_command_palette(
    store: &Arc<Mutex<WorkspaceState>>,
    version: Signal<u64>,
    palette: &CommandPaletteState,
    matches: &[CommandPaletteItem],
    accent_hex: &'static str,
    accent_tint: &'static str,
    xray: bool,
    m: LayoutMetrics,
) -> Element {
    let (card_bw, card_bc) = card_border(xray);
    let visible = &matches[..matches.len().min(MAX_VISIBLE_PALETTE_ITEMS)];
    let selected = if visible.is_empty() {
        0
    } else {
        palette.selected.min(visible.len() - 1)
    };
    let count_lbl = format!("{} matches", matches.len());

    let s_close = Arc::clone(store);
    let s_input = Arc::clone(store);
    let s_submit = Arc::clone(store);
    let s_key = Arc::clone(store);

    rsx! {
        view {
            test_id: "command-palette",
            accessibility_label: "Command Palette",
            accessibility_role: "dialog",
            style: "width: 100%; flex-shrink: 0; flex-direction: column; padding: 8px; gap: 6px; background-color: {PANEL_BG}; border-radius: {m.card_radius}px; border-width: 2px; border-color: {accent_hex};",

            // Header row
            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center;",
                view {
                    style: "flex-direction: row; align-items: center; gap: 6px;",
                    text { style: "color: {accent_hex}; font-size: 11px;", "CMD+K PALETTE" }
                    text { style: "color: {TEXT_MUTED}; font-size: 10px;", "{count_lbl}" }
                }
                pressable {
                    test_id: "palette-close",
                    accessibility_label: "Close command palette",
                    style: "width: 40px; height: 20px; flex-shrink: 0; justify-content: center; align-items: center; background-color: {CONTROL_BG}; border-radius: {m.pill_radius}px; border-width: 1px; border-color: {BORDER_DEFAULT};",
                    onpress: move |_| mutate_state(&s_close, version, |st| st.close_palette()),
                    text { style: "color: {TEXT_SECONDARY}; font-size: 9px;", "Esc" }
                }
            }

            // Live Fuzzy Search TextInput
            textinput {
                test_id: "palette-input",
                placeholder: "Jump to view, task, accent, layout, snapshot...",
                value: "{palette.query}",
                multiline: false,
                accessibility_label: "Command palette filter",
                accessibility_role: "searchbox",
                style: "width: 100%; height: 28px; flex-shrink: 0; padding-horizontal: 8px; background-color: {CARD_ALT_BG}; color: {TEXT_PRIMARY}; font-size: 11px; border-radius: {m.button_radius}px; border-width: {card_bw}px; border-color: {card_bc};",
                onchange: move |ev| {
                    let q = ev.data().value.clone();
                    mutate_state(&s_input, version, |st| st.set_palette_query(q));
                },
                onsubmit: move |_| {
                    mutate_state(&s_submit, version, |st| {
                        let _ = st.execute_palette_selected();
                    });
                },
                onkeydown: move |ev| {
                    let data = ev.data();
                    let k = data.key.clone();
                    let mod_key = data.meta || data.ctrl;
                    mutate_state(&s_key, version, |st| {
                        let _ = dispatch_keydown(st, &k, mod_key);
                    });
                },
            }

            // Filtered Results
            if visible.is_empty() {
                view {
                    test_id: "palette-empty",
                    style: "width: 100%; height: 26px; justify-content: center; align-items: center; background-color: {CARD_ALT_BG}; border-radius: {m.pill_radius}px;",
                    text { style: "color: {TEXT_MUTED}; font-size: 10px;", "No matching commands" }
                }
            } else {
                view {
                    style: "width: 100%; flex-shrink: 0; flex-direction: column; gap: 4px;",
                    for (idx, item) in visible.iter().enumerate() {
                        {
                            let is_sel = idx == selected;
                            let bg = if is_sel { accent_tint } else { CARD_BG };
                            let fg = if is_sel { TEXT_PRIMARY } else { TEXT_SECONDARY };
                            let (bw, bc) = control_border(xray, is_sel, accent_hex);
                            let item_tid = format!("palette-item-{idx}");
                            let s_exec = Arc::clone(store);
                            rsx! {
                                pressable {
                                    key: "pal-{idx}",
                                    test_id: "{item_tid}",
                                    style: "width: 100%; height: 26px; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; padding-horizontal: 8px; background-color: {bg}; border-radius: {m.pill_radius}px; border-width: {bw}px; border-color: {bc};",
                                    onpress: move |_| mutate_state(&s_exec, version, |st| {
                                        let _ = st.execute_palette_at(idx);
                                    }),
                                    view {
                                        style: "flex-direction: row; align-items: center; gap: 6px; flex: 1;",
                                        view {
                                            style: "height: 16px; flex-shrink: 0; justify-content: center; align-items: center; padding-horizontal: 5px; background-color: {CONTROL_BG}; border-radius: 4px;",
                                            text { style: "color: {accent_hex}; font-size: 9px;", "{item.category}" }
                                        }
                                        text { style: "color: {fg}; font-size: 10px;", "{item.label}" }
                                    }
                                    if is_sel {
                                        text { style: "color: {accent_hex}; font-size: 9px;", "Enter" }
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
