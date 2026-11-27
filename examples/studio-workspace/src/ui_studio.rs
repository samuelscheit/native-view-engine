//! Docked / drawer **NVE Studio** live self-editing meta-workbench panel.
//!
//! Built with shared `nve-ui` primitives (`Card`, `Button`, `Badge`, `Separator`), exposing
//! Accent swatches, Corner Radius, Density, Live Taffy <-> Yoga Layout Engine switcher,
//! X-Ray HUD, Live Sync & multi-peer binary `NVC1` CRDT telemetry (with Sync Now and
//! Partition/Heal controls), and deterministic Time-Travel Scrubber.
//!
//! [`StudioPanel`] subscribes to the `studio` domain; [`SyncCard`] to `sync` and
//! [`TimeTravelCard`] to `inspector`, so telemetry ticks and history scrubs repaint only their card.

use nve_dioxus::prelude::*;
use nve_dioxus::traits::TraitScope;
use nve_ui::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, ColorHex, Separator};

use crate::state::{AccentTheme, CornerRadiusMode, DensityMode, LayoutEngineMode, WorkspaceState};
use crate::theme::badge_min_w;
use crate::traits::{studio_compact_override, StudioTraitPreviewChip};
use crate::ui::store::{use_studio, StudioStore};
use dioxus_core::EventHandler;
use crate::ui::style::{data_attrs, use_studio_style, StudioStyle};

fn section_style(s: &StudioStyle) -> String {
    format!(
        "width: 100%; flex-shrink: 0; padding: 5px; gap: 3px; background-color: {}; border-radius: {}px; {}",
        s.t.surface.sunken.hex(),
        s.m.button_radius,
        s.card()
    )
}

fn segment_style(s: &StudioStyle, selected: bool) -> String {
    s.compact_control("flex: 1; height: 20px; min-height: 20px;", 2, s.m.pill_radius, Some(selected))
}

/// One row of mutually exclusive Studio options; `apply` mutates the workspace for the pressed one.
#[component]
fn OptionRow(options: Vec<(&'static str, &'static str, bool)>, apply: EventHandler<usize>) -> Element {
    let s = use_studio_style();
    rsx! {
        view {
            style: "width: 100%; flex-direction: row; gap: 4px;",
            for (idx, (tid, label, selected)) in options.into_iter().enumerate() {
                Button {
                    key: "{tid}",
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some(tid.into()),
                    style: Some(segment_style(&s, selected)),
                    on_press: move |_| apply.call(idx),
                    "{label}"
                }
            }
        }
    }
}

/// `OptionRow` options for `choices` with the `current` one selected.
fn options<T: Copy + PartialEq>(
    choices: &[(T, &'static str, &'static str)],
    current: T,
) -> Vec<(&'static str, &'static str, bool)> {
    choices.iter().map(|&(v, tid, label)| (tid, label, v == current)).collect()
}

/// Applies `choices[idx]` through `set`.
fn apply_choice<T: Copy>(
    store: StudioStore,
    choices: &[(T, &'static str, &'static str)],
    idx: usize,
    set: fn(&mut WorkspaceState, T),
) {
    if let Some(&(v, _, _)) = choices.get(idx) {
        store.update(|st| set(st, v));
    }
}

const ACCENTS: &[(AccentTheme, &str, &str)] = &[
    (AccentTheme::Cyan, "accent-cyan", "Cyan"),
    (AccentTheme::Violet, "accent-violet", "Violet"),
    (AccentTheme::Green, "accent-green", "Green"),
    (AccentTheme::Amber, "accent-amber", "Amber"),
];
const RADII: &[(CornerRadiusMode, &str, &str)] = &[
    (CornerRadiusMode::Sharp, "radius-sharp", "Sharp"),
    (CornerRadiusMode::Soft, "radius-soft", "Soft"),
    (CornerRadiusMode::Pill, "radius-pill", "Pill"),
];
const DENSITIES: &[(DensityMode, &str, &str)] = &[
    (DensityMode::Normal, "density-normal", "Normal"),
    (DensityMode::Compact, "density-compact", "Compact"),
];
const ENGINES: &[(LayoutEngineMode, &str, &str)] = &[
    (LayoutEngineMode::Taffy, "layout-taffy", "Taffy"),
    (LayoutEngineMode::Yoga, "layout-yoga", "Yoga"),
];

#[component]
pub(crate) fn StudioPanel(mobile: bool) -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let config = store.studio.read().config.clone();
    let engine = config.layout_engine;
    let size = if mobile { "width: 100%;" } else { "width: 210px; height: 100%;" };
    let section = section_style(&s);
    let caption = t.text.tertiary.hex();
    let xray = config.xray;

    rsx! {
        view {
            test_id: "studio-panel",
            nve_table: data_attrs([("layout-engine", engine.as_str().to_owned())]),
            style: "{size} flex-shrink: 0; flex-direction: column; padding: 7px; gap: 5px; background-color: {t.surface.base.hex()}; {s.panel()}",

            view {
                style: "width: 100%; flex-shrink: 0; flex-direction: row; justify-content: space-between; align-items: center; {s.row()}",
                text { style: "color: {t.text.primary.hex()}; font-size: 12px;", "NVE STUDIO" }
                Badge {
                    label: "LIVE SELF-EDIT".to_string(),
                    variant: BadgeVariant::Accent,
                    style: Some(s.compact_control(
                        &format!("min-width: {}px; height: 17px;", badge_min_w("LIVE SELF-EDIT")),
                        4,
                        s.m.pill_radius,
                        None,
                    )),
                }
            }

            // Live environment readout (inherits the root TraitProvider; the compact preview uses TraitScope).
            StudioTraitPreviewChip { test_id: Some("studio-trait-live".into()) }
            if !mobile {
                TraitScope {
                    overrides: studio_compact_override(true),
                    StudioTraitPreviewChip { test_id: Some("studio-trait-scoped".into()) }
                }
            }

            Card {
                variant: CardVariant::Subtle,
                padding: Some(5.0),
                gap: Some(3.0),
                style: Some(section.clone()),
                text { style: "color: {caption}; font-size: 9px;", "ACCENT" }
                OptionRow {
                    options: options(&ACCENTS[..2], config.accent),
                    apply: move |idx| apply_choice(store, ACCENTS, idx, WorkspaceState::set_accent),
                }
                OptionRow {
                    options: options(&ACCENTS[2..], config.accent),
                    apply: move |idx| apply_choice(store, &ACCENTS[2..], idx, WorkspaceState::set_accent),
                }
            }

            Card {
                variant: CardVariant::Subtle,
                padding: Some(5.0),
                gap: Some(3.0),
                style: Some(section.clone()),
                text { style: "color: {caption}; font-size: 9px;", "RADIUS (0 / 12 / 18)" }
                OptionRow {
                    options: options(RADII, config.radius),
                    apply: move |idx| apply_choice(store, RADII, idx, WorkspaceState::set_radius),
                }
                text { style: "color: {caption}; font-size: 9px;", "DENSITY" }
                OptionRow {
                    options: options(DENSITIES, config.density),
                    apply: move |idx| apply_choice(store, DENSITIES, idx, WorkspaceState::set_density),
                }
            }

            Card {
                variant: CardVariant::Subtle,
                padding: Some(5.0),
                gap: Some(3.0),
                style: Some(section),
                text { style: "color: {caption}; font-size: 9px;", "LAYOUT ENGINE" }
                OptionRow {
                    options: options(ENGINES, engine),
                    apply: move |idx| apply_choice(store, ENGINES, idx, WorkspaceState::set_layout_engine),
                }
                view {
                    test_id: "layout-engine-readout",
                    nve_table: data_attrs([("layout-engine", engine.as_str().to_owned())]),
                    style: "width: 100%; flex-shrink: 0;",
                    text { style: "color: {t.text.link.hex()}; font-size: 9px;", "{engine.readout_label()}" }
                }
            }

            Button {
                variant: ButtonVariant::Secondary,
                size: ButtonSize::Sm,
                test_id: Some("toggle-xray".into()),
                style: Some(s.compact_control("width: 100%; height: 24px; min-height: 24px;", 4, s.m.button_radius, Some(xray))),
                on_press: move |_| store.update(|st| st.toggle_xray()),
                if xray { "X-Ray HUD: ON" } else { "X-Ray HUD: OFF" }
            }

            SyncCard {}
            TimeTravelCard {}
        }
    }
}

/// Live sync / pairing + binary `NVC1` CRDT mesh telemetry.
#[component]
fn SyncCard() -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let sync = store.sync.read().telemetry.clone();
    let peers = format!("{} {}", sync.connected_peers, if sync.connected_peers == 1 { "peer" } else { "peers" });
    let metrics = format!("{} | {} | {}", sync.format_kbps(), sync.format_compression(), sync.format_rtt());
    let crdt_summary = format!("NVC1 {}/{}d | {}B", sync.crdt_deltas_sent, sync.crdt_deltas_received, sync.crdt_bytes_synced);
    let split = sync.crdt_partitioned;
    let status_lbl = if split { "SPLIT" } else { "MESH" };
    let transport = sync.transport.label();

    rsx! {
        Card {
            variant: CardVariant::Subtle,
            test_id: Some("studio-sync-telemetry".into()),
            padding: Some(5.0),
            gap: Some(3.0),
            style: Some(section_style(&s)),
            view {
                nve_table: data_attrs([("sync-transport", sync.transport.as_str().to_owned())]),
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                text { style: "color: {t.text.tertiary.hex()}; font-size: 9px;", "LIVE SYNC" }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("sync-transport-cycle".into()),
                    accessibility_label: Some("Cycle live sync transport mode".into()),
                    style: Some(s.compact_control(
                        &format!("min-width: {}px; height: 17px; min-height: 17px;", badge_min_w(transport)),
                        4,
                        s.m.pill_radius,
                        Some(true),
                    )),
                    on_press: move |_| store.update(|st| st.cycle_sync_transport()),
                    "{transport}"
                }
            }
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                text { style: "color: {t.text.primary.hex()}; font-size: 9px;", "{peers}" }
                text { style: "color: {t.text.secondary.hex()}; font-size: 9px;", "{metrics}" }
            }
            Separator {}
            view {
                test_id: "studio-crdt-metrics",
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center;",
                text { style: "color: {t.text.link.hex()}; font-size: 9px;", "{crdt_summary}" }
                Badge {
                    label: status_lbl.to_string(),
                    variant: if split { BadgeVariant::Danger } else { BadgeVariant::Success },
                    style: Some(s.compact_control(
                        &format!("min-width: {}px; height: 15px;", badge_min_w(status_lbl)),
                        3,
                        s.m.pill_radius,
                        None,
                    )),
                }
            }
            view {
                style: "width: 100%; flex-direction: row; gap: 4px;",
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("crdt-sync-now".into()),
                    accessibility_label: Some("Sync binary CRDT deltas now".into()),
                    style: Some(segment_style(&s, true)),
                    on_press: move |_| store.update(|st| st.sync_crdt_now()),
                    "Sync Now"
                }
                Button {
                    variant: if split { ButtonVariant::Primary } else { ButtonVariant::Destructive },
                    size: ButtonSize::Sm,
                    test_id: Some("crdt-toggle-partition".into()),
                    accessibility_label: Some("Simulate or heal CRDT network partition".into()),
                    style: Some(s.compact_control("flex: 1; height: 20px; min-height: 20px;", 2, s.m.pill_radius, None)),
                    on_press: move |_| store.update(|st| st.toggle_crdt_partition()),
                    if split { "Heal Mesh" } else { "Partition" }
                }
            }
        }
    }
}

/// Deterministic time-travel scrubber.
#[component]
fn TimeTravelCard() -> Element {
    let store = use_studio();
    let s = use_studio_style();
    let t = s.t;
    let i = store.inspector.read().clone();
    let step = |enabled: bool| {
        let style = segment_style(&s, false);
        if enabled { style } else { format!("{style} opacity: 0.4;") }
    };
    let (undo_style, redo_style) = (step(i.can_undo), step(i.can_redo));

    rsx! {
        Card {
            variant: CardVariant::Subtle,
            padding: Some(5.0),
            gap: Some(3.0),
            style: Some(section_style(&s)),
            text { style: "color: {t.text.tertiary.hex()}; font-size: 9px;", "TIME TRAVEL" }
            text { style: "color: {t.text.primary.hex()}; font-size: 10px;", "Snapshot {i.snapshot_index} / {i.history_len}" }
            text { style: "color: {t.text.link.hex()}; font-size: 9px;", "{i.action_label}" }
            view {
                style: "width: 100%; flex-direction: row; gap: 4px;",
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("tt-first".into()),
                    style: Some(undo_style.clone()),
                    on_press: move |_| store.update(|st| st.scrub_first()),
                    "<< #1"
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("tt-undo".into()),
                    style: Some(undo_style),
                    on_press: move |_| store.update(|st| st.scrub_undo()),
                    "< Undo"
                }
            }
            view {
                style: "width: 100%; flex-direction: row; gap: 4px;",
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("tt-redo".into()),
                    style: Some(redo_style.clone()),
                    on_press: move |_| store.update(|st| st.scrub_redo()),
                    "Redo >"
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    test_id: Some("tt-live".into()),
                    style: Some(redo_style),
                    on_press: move |_| store.update(|st| st.scrub_live()),
                    "Live >>"
                }
            }
        }
    }
}
